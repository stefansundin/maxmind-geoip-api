#![allow(clippy::needless_return)]

use bytes::Buf;
use log::{debug, error, info};
use std::{
  env,
  error::Error,
  fs,
  path::{Path, PathBuf},
  process,
  sync::OnceLock,
  time,
};

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
          error!("Error: {} is not a directory", &data_dir);
          process::exit(1);
        }
      }
      Err(err) => {
        error!("Error: {}: {}", &data_dir, err);
        process::exit(1);
      }
    }

    data_dir
  })
}

pub fn database_path() -> &'static Path {
  static DATABASE_PATH: OnceLock<PathBuf> = OnceLock::new();
  DATABASE_PATH.get_or_init(|| Path::new(data_dir()).join("database.mmdb"))
}

pub fn batch_limit() -> &'static usize {
  static BATCH_LIMIT: OnceLock<usize> = OnceLock::new();
  BATCH_LIMIT.get_or_init(|| {
    env::var("BATCH_LIMIT")
      .ok()
      .and_then(|v| v.parse().ok())
      .unwrap_or(1000)
  })
}

fn save_mmdb(
  source_path: &Path,
  temp_path: &Path,
  destination_path: &Path,
) -> Result<usize, Box<dyn Error>> {
  // This function pulls out the mmdb file from a bunch of possible compression formats, even combinations that are unlikely
  // So it needs two temporary files to do this without putting everything in memory
  // At the end the mmdb file is moved to the destination path, in an atomic operation
  let mut read_path = source_path;
  let mut write_path = temp_path;

  // Return how many layers deep the mmdb file was
  // This is used to determine whether or not to output "Extracting mmdb file" statistics
  let mut how_deep = 0;

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
        let path = path.to_str().unwrap_or("");
        if path.starts_with("__MACOSX/") {
          continue;
        }
        if path.ends_with(".mmdb") {
          std::io::copy(&mut file, &mut writer)?;
          writer.sync_data()?;
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
      writer.sync_data()?;
      fs::remove_file(read_path)?;
    } else if fmt == file_format::FileFormat::Bzip2 {
      // .bz2
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut decompressor = bzip2::read::BzDecoder::new(reader);
      std::io::copy(&mut decompressor, &mut writer)?;
      writer.sync_data()?;
      fs::remove_file(read_path)?;
    } else if fmt == file_format::FileFormat::Zip {
      // .zip
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut archive = zip::ZipArchive::new(reader)?;
      let mut found = false;
      for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let name = file.name();
        if name.starts_with("__MACOSX/") {
          continue;
        }
        if name.ends_with(".mmdb") {
          std::io::copy(&mut file, &mut writer)?;
          writer.sync_data()?;
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
      writer.sync_data()?;
      fs::remove_file(read_path)?;
    } else if fmt == file_format::FileFormat::Zstandard {
      // .zst
      let reader = fs::File::open(read_path)?;
      let mut writer = fs::File::create(write_path)?;
      let mut decompressor = zstd::Decoder::new(reader)?;
      std::io::copy(&mut decompressor, &mut writer)?;
      writer.sync_data()?;
      fs::remove_file(read_path)?;
    } else {
      break;
    }

    std::mem::swap(&mut read_path, &mut write_path);
    how_deep += 1;
  }

  // verify that the database can be opened successfully
  unsafe {
    match maxminddb::Reader::open_mmap(read_path) {
      Ok(reader) => {
        debug!("{:?}", reader.metadata);
      }
      Err(err) => {
        fs::remove_file(read_path)?;
        return Err(format!("Error opening newly downloaded database: {}", err).into());
      }
    }
  }

  fs::rename(read_path, destination_path)?;
  Ok(how_deep)
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

/// Returns Ok(true) if a new database was downloaded, otherwise Ok(false) usually means the remote server didn't have a new database file.
pub async fn download_database(force: bool) -> Result<bool, Box<dyn Error>> {
  let database_path = database_path();
  let url = match env::var("MAXMIND_DB_URL") {
    Ok(url) => url,
    Err(_) => {
      return Err(
        format!(
          "Please configure MAXMIND_DB_URL or place a database file at {}",
          database_path.display()
        )
        .into(),
      );
    }
  };

  // The stamp file keeps track of when a download request was last performed
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
      if duration_since < crate::UPDATE_CHECK_INTERVAL {
        let formatter = timeago::Formatter::new();
        let formatted_time = formatter.convert(duration_since);
        info!(
          "Last checked for a database update {}, skipping check",
          formatted_time
        );
        return Ok(false);
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

  let download_start_time = time::Instant::now();
  let response = request.send().await?;
  let download_duration = download_start_time.elapsed();

  // Touch stamp file
  fs::write(stamp_path, "")?;

  match response.status() {
    reqwest::StatusCode::OK => (),
    reqwest::StatusCode::NOT_MODIFIED => {
      info!("The database is up to date");
      return Ok(false);
    }
    status_code => {
      return Err(format!("Unexpected response code: {}", status_code).into());
    }
  }
  debug!("Downloading file took: {:?}", download_duration);

  let etag = response
    .headers()
    .get("ETag")
    .and_then(|v| v.to_str().ok().map(|v| v.to_string()));

  let temp_path = Path::new(data_dir()).join("database.mmdb.temp");
  let temp_path2 = Path::new(data_dir()).join("database.mmdb.temp2");
  let mut temp_file = fs::File::create(&temp_path)?;
  let mut reader = response.bytes().await?.reader();
  std::io::copy(&mut reader, &mut temp_file)?;
  temp_file.sync_data()?;

  let extract_start_time = time::Instant::now();
  match save_mmdb(&temp_path, &temp_path2, database_path) {
    Ok(how_deep) => {
      if how_deep > 0 {
        let extract_duration = extract_start_time.elapsed();
        debug!("Extracting mmdb file took: {:?}", extract_duration);
      }
    }
    Err(err) => return Err(err),
  }

  if let Some(etag) = etag {
    fs::write(etag_path, etag)?;
  }

  info!("Downloaded a database");

  Ok(true)
}
