use actix_web::web::Buf;
use chrono::{TimeZone, Utc};
use log::{debug, error, info, warn};
use std::sync::OnceLock;
use std::{
  env,
  error::Error,
  fs,
  path::{Path, PathBuf},
  process, time,
};
use zip::ZipArchive;

pub fn get_env_var(name: &str) -> String {
  match env::var(name) {
    Ok(v) => v,
    Err(err) => {
      error!("{}: {}", name, err);
      process::exit(1);
    }
  }
}

pub fn data_dir() -> &'static str {
  static DATA_DIR: OnceLock<String> = OnceLock::new();
  DATA_DIR.get_or_init(|| {
    let data_dir = get_env_var("DATA_DIR");

    match fs::metadata(&data_dir) {
      Ok(metadata) => {
        if !metadata.is_dir() {
          eprintln!("Error: {} is not a directory", &data_dir);
          process::exit(1);
        }
      }
      Err(err) => {
        eprintln!("Error: {}: {}", &data_dir, err);
        process::exit(1);
      }
    }

    return data_dir;
  })
}

pub fn database_path() -> &'static Path {
  static DATABASE_PATH: OnceLock<PathBuf> = OnceLock::new();
  DATABASE_PATH.get_or_init(|| Path::new(data_dir()).join("database.mmdb"))
}

fn save_mmdb(
  source_path: &Path,
  temp_path: &Path,
  destination_path: &Path,
) -> Result<(), Box<dyn Error>> {
  // This function pulls out the mmdb file from a bunch of possible compression formats, even combinations that are unlikely
  // So it needs two temporary files to do this without putting everything in memory
  // At the end the mmdb file is moved to the destination path, in an atomic operation
  let mut read_path = source_path;
  let mut write_path = temp_path;

  loop {
    let fmt = file_format::FileFormat::from_file(read_path)?;

    if fmt == file_format::FileFormat::TapeArchive {
      // .tar
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut archive = tar::Archive::new(reader);
      let mut found = false;
      for file in archive.entries()? {
        let mut file = file?;
        let path = file.path()?;
        let path = path.to_str().unwrap_or(&"");
        if path.starts_with("__MACOSX/") {
          continue;
        }
        if path.ends_with(".mmdb") {
          std::io::copy(&mut file, &mut writer)?;
          writer.sync_all()?;
          fs::remove_file(read_path)?;
          found = true;
          break;
        }
      }
      if !found {
        return Err("mmdb file not found in archive".into());
      }
    } else if fmt == file_format::FileFormat::Gzip {
      // .gz
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut decompressor = flate2::read::GzDecoder::new(reader);
      std::io::copy(&mut decompressor, &mut writer)?;
      writer.sync_all()?;
      fs::remove_file(read_path)?;
    } else if fmt == file_format::FileFormat::Gzip {
      // .bz2
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut decompressor = bzip2::read::BzDecoder::new(reader);
      std::io::copy(&mut decompressor, &mut writer)?;
      writer.sync_all()?;
      fs::remove_file(read_path)?;
    } else if fmt == file_format::FileFormat::Zip {
      // .zip
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut archive = ZipArchive::new(reader)?;
      let mut found = false;
      for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name();
        if name.starts_with("__MACOSX/") {
          continue;
        }
        if name.ends_with(".mmdb") {
          std::io::copy(&mut file, &mut writer)?;
          writer.sync_all()?;
          fs::remove_file(read_path)?;
          found = true;
          break;
        }
      }
      if !found {
        return Err("mmdb file not found in archive".into());
      }
    } else if fmt == file_format::FileFormat::Xz {
      // .xz
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut decompressor = xz2::read::XzDecoder::new(reader);
      std::io::copy(&mut decompressor, &mut writer)?;
      writer.sync_all()?;
      fs::remove_file(read_path)?;
    } else {
      break;
    }

    std::mem::swap(&mut read_path, &mut write_path);
  }

  // verify that the database can be opened successfully
  match maxminddb::Reader::open_mmap(&read_path) {
    Ok(reader) => {
      debug!("{:?}", reader.metadata);
    }
    Err(err) => {
      fs::remove_file(read_path)?;
      return Err(format!("Error opening newly downloaded database: {}", err).into());
    }
  }

  fs::rename(&read_path, destination_path)?;
  Ok(())
}

fn build_reqwest_client() -> Result<reqwest::Client, reqwest::Error> {
  let mut builder = reqwest::Client::builder();

  if let Ok(v) = env::var("CA_BUNDLE") {
    let cert_data = std::fs::read(v).expect("error reading CA_BUNDLE file");
    let cert = reqwest::Certificate::from_pem(&cert_data)?;
    builder = builder.add_root_certificate(cert);
  }

  if let Ok(v) = env::var("DANGER_ACCEPT_INVALID_CERTS") {
    builder = builder.danger_accept_invalid_certs(v == "true" || v == "1");
  }

  return builder.build();
}

pub async fn download_database(force: bool) -> Result<(), Box<dyn Error>> {
  let database_path = database_path();
  let url = env::var("MAXMIND_DB_URL");
  if url.is_err() {
    if database_path.is_file() {
      return Ok(());
    } else {
      error!(
        "Please configure MAXMIND_DB_URL or place a database file at {}",
        database_path.display()
      );
      process::exit(1);
    }
  }

  let url = url.unwrap();
  let stamp_path = Path::new(data_dir()).join("stamp");

  // Skip check if we have a downloaded database already and it has been less than 24 hours since the last check
  if !force && database_path.is_file() && stamp_path.is_file() {
    if let Ok(metadata) = fs::metadata(&stamp_path) {
      let modified_date = metadata
        .modified()
        .expect("error getting stamp last modified date");
      let duration_since = time::SystemTime::now()
        .duration_since(modified_date)
        .expect("error calculating time duration since stamp last modified date");
      let one_day = time::Duration::from_secs(24 * 60 * 60);
      if duration_since < one_day {
        let formatter = timeago::Formatter::new();
        let formatted_time = formatter.convert(duration_since);
        info!(
          "Last checked for a database update {}, skipping check.",
          formatted_time
        );
        return Ok(());
      }
    }
  }

  let mut request = build_reqwest_client()?.get(&url);
  let etag_path = Path::new(data_dir()).join("etag");
  if database_path.is_file() && etag_path.is_file() {
    if let Ok(etag) = fs::read_to_string(&etag_path) {
      request = request.header("If-None-Match", etag);
    }
  }
  let response = request.send().await?;

  let status_code = response.status();
  if status_code == reqwest::StatusCode::NOT_MODIFIED {
    info!("The database file is up to date.");
    fs::write(stamp_path, "")?;
    return Ok(());
  } else if status_code != reqwest::StatusCode::OK {
    if database_path.is_file() {
      warn!("Got unexpected response code: {}", status_code);
      match fs::metadata(&database_path) {
        Ok(metadata) => {
          let modified_date = metadata
            .modified()
            .expect("error getting database last modified date");
          let duration_since = time::SystemTime::now()
            .duration_since(modified_date)
            .expect("error calculating time duration since database last modified date");
          let formatter = timeago::Formatter::new();
          let formatted_time = formatter.convert(duration_since);
          info!(
            "There is a database saved from {} so ignoring the error.",
            formatted_time
          );
          return Ok(());
        }
        Err(err) => {
          return Err(format!("Error: {:?}: {}", &database_path, err).into());
        }
      }
    } else {
      return Err(format!("Got unexpected response code: {}", status_code).into());
    }
  }

  let etag = response.headers().get("ETag").map(|v| v.clone());

  let temp_path = Path::new(data_dir()).join("database.mmdb.temp");
  let temp_path2 = Path::new(data_dir()).join("database.mmdb.temp2");
  let mut temp_file = fs::File::create(&temp_path)?;
  let mut reader = response.bytes().await?.reader();
  // why does this copy require a trait from actix_web??
  std::io::copy(&mut reader, &mut temp_file)?;
  temp_file.sync_all()?;

  if let Err(err) = save_mmdb(&temp_path, &temp_path2, &database_path) {
    if database_path.is_file() {
      warn!("{}", err);
      return Ok(());
    } else {
      return Err(err);
    }
  }

  if let Some(etag) = etag {
    fs::write(
      etag_path,
      etag.to_str().expect("error converting ETag to string"),
    )?;
  }

  fs::write(stamp_path, "")?;

  let db = maxminddb::Reader::open_mmap(&database_path)?;
  let datetime = Utc
    .timestamp_opt(db.metadata.build_epoch.try_into()?, 0)
    .unwrap();
  info!(
    "Downloaded a database ({} dated {})",
    db.metadata.database_type,
    datetime.format("%Y-%m-%d")
  );

  Ok(())
}
