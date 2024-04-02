use actix_web::web::Buf;
use log::{error, info, warn};
use std::io::Read;
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

pub async fn download_database() -> Result<(), Box<dyn Error>> {
  let database_path = database_path();
  let etag_path = Path::new(data_dir()).join("etag");
  let url = get_env_var("MAXMIND_DB_URL");

  let mut request = reqwest::Client::new().get(url);
  if database_path.is_file() && etag_path.is_file() {
    if let Ok(etag) = fs::read_to_string(&etag_path) {
      request = request.header("If-None-Match", etag);
    }
  }
  let response = request.send().await.expect("error fetching database file");

  let status_code = response.status();
  if status_code == reqwest::StatusCode::NOT_MODIFIED {
    info!("The database file is up to date.");
    return Ok(());
  } else if status_code != reqwest::StatusCode::OK {
    if database_path.is_file() {
      warn!("Got unexpected response code: {}", status_code);
      match fs::metadata(&database_path) {
        Ok(metadata) => {
          let modified_date = metadata
            .modified()
            .expect("error getting database last modified date");
          let duration = time::SystemTime::now()
            .duration_since(modified_date)
            .expect("error calculating time duration since database last modified date");
          let formatter = timeago::Formatter::new();
          let formatted_time = formatter.convert(duration);
          info!(
            "There is a database saved from {} so ignoring the error",
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

  let etag = response
    .headers()
    .get("ETag")
    .expect("read ETag header")
    .clone();
  let content_type = response
    .headers()
    .get("Content-Type")
    .expect("read Content-Type header")
    .clone();
  let last_modified = response
    .headers()
    .get("Last-Modified")
    .expect("read Last-Modified header")
    .clone();

  // Download the new database to a temporary file and then rename it to perform an atomic replacement of the old database
  let temp_path = Path::new(data_dir()).join("database.mmdb.temp");
  let mut output_file = fs::File::create(&temp_path)?;
  // why does this copy require a trait from actix_web??
  let mut reader = response.bytes().await?.reader();
  if content_type == "application/gzip" {
    // This does not handle .tar.gz files
    let mut decompressor = flate2::read::GzDecoder::new(reader);
    std::io::copy(&mut decompressor, &mut output_file)?;
  } else if content_type == "application/x-bzip2" {
    // This does not handle .tar.bz2 files
    let mut decompressor = bzip2::read::BzDecoder::new(reader);
    std::io::copy(&mut decompressor, &mut output_file)?;
  } else if content_type == "application/x-xz" {
    let mut decompressor = xz2::read::XzDecoder::new(reader);
    std::io::copy(&mut decompressor, &mut output_file)?;
  } else if content_type == "application/zip" {
    let mut all_bytes = Vec::new();
    reader.read_to_end(&mut all_bytes)?;
    let seekable_reader = std::io::Cursor::new(all_bytes);
    let mut archive = ZipArchive::new(seekable_reader)?;
    let mut found = false;
    for i in 0..archive.len() {
      let mut file = archive.by_index(i)?;
      let name = file.name();
      if name.starts_with("__MACOSX/") {
        continue;
      }
      if name.to_lowercase().ends_with(".mmdb") {
        std::io::copy(&mut file, &mut output_file)?;
        found = true;
        break;
      }
    }
    if !found {
      return Err("no .mmdb file found in archive".into());
    }
  } else {
    std::io::copy(&mut reader, &mut output_file)?;
  }
  std::fs::rename(&temp_path, database_path)?;
  std::fs::write(
    etag_path,
    etag.to_str().expect("error converting ETag to string"),
  )?;

  info!(
    "Downloaded a new database (Last-Modified: {})",
    last_modified
      .to_str()
      .expect("convert Last-Modified to string")
  );

  Ok(())
}
