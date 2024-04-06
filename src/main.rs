use actix_web::{get, middleware, web, App, HttpResponse, HttpServer};
use chrono::{TimeZone, Utc};
use log::info;
use log::{debug, error};
use maxminddb::{geoip2, Metadata, Mmap, Reader};
use serde::Serialize;
use serde_json::json;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::{collections::BTreeMap, env, net::IpAddr, process};
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{interval, Duration};

pub mod utils;

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");

fn load_database() -> Reader<Mmap> {
  let reader = Reader::open_mmap(utils::database_path()).expect("error opening database");
  let datetime = Utc
    .timestamp_opt(
      reader
        .metadata
        .build_epoch
        .try_into()
        .expect("parsing build_epoch"),
      0,
    )
    .unwrap();
  info!(
    "Loaded a {} database dated {}",
    reader.metadata.database_type,
    datetime.format("%Y-%m-%d")
  );
  return reader;
}

fn reader_lock() -> &'static RwLock<Reader<Mmap>> {
  static READER_LOCK: OnceLock<RwLock<Reader<Mmap>>> = OnceLock::new();
  READER_LOCK.get_or_init(|| RwLock::new(load_database()))
}

fn reload_database() {
  let new_reader = load_database();
  let mut reader = reader_lock()
    .write()
    .expect("error getting write-access to reader");
  *reader = new_reader;
}

#[derive(Serialize)]
#[serde(remote = "Metadata")]
struct MetadataDef {
  binary_format_major_version: u16,
  binary_format_minor_version: u16,
  build_epoch: u64,
  database_type: String,
  description: BTreeMap<String, String>,
  ip_version: u16,
  languages: Vec<String>,
  node_count: u32,
  record_size: u16,
}

#[derive(Serialize)]
struct MetadataWrapper<'a>(#[serde(with = "MetadataDef")] &'a Metadata);

#[get("/metadata")]
async fn metadata() -> Result<HttpResponse, actix_web::error::Error> {
  let reader = reader_lock().read().expect("error getting reader");
  debug!("{:?}", reader.metadata);

  return Ok(
    HttpResponse::Ok()
      .append_header(("content-type", "application/json"))
      .body(json!(&MetadataWrapper(&reader.metadata)).to_string()),
  );
}

#[get("/{ip}")]
async fn lookup(addr: web::Path<IpAddr>) -> Result<HttpResponse, actix_web::error::Error> {
  let addr = addr.into_inner();
  debug!("addr: {}", addr);

  let reader = reader_lock().read().expect("error getting reader");
  let result: Result<geoip2::City, _> = reader.lookup(addr);
  let city = match result {
    Ok(city) => city,
    Err(_) => return Ok(HttpResponse::NotFound().finish()),
  };
  debug!("city: {:?}", city);

  return Ok(
    HttpResponse::Ok()
      .append_header(("content-type", "application/json"))
      .append_header(("x-maxmind-build-epoch", reader.metadata.build_epoch))
      .body(json!(city).to_string()),
  );
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

  let version = VERSION.unwrap_or("unknown");
  info!("version {}", version);

  // Send the process a SIGHUP to download a new database
  tokio::spawn(async {
    let mut sighup = signal(SignalKind::hangup()).expect("error listening for SIGHUP");
    while let Some(_) = sighup.recv().await {
      match utils::download_database(true).await {
        Ok(_) => reload_database(),
        Err(err) => error!("Error downloading new database: {:?}", err),
      }
    }
  });

  if let Err(err) = utils::download_database(false).await {
    error!("Error downloading database: {:?}", err);
    process::exit(1);
  }

  // Load the database
  reader_lock();

  // Check for database updates every 24 hours
  if env::var("MAXMIND_DB_URL").is_ok() {
    tokio::spawn(async {
      let mut interval = interval(Duration::from_secs(24 * 60 * 60));
      interval.tick().await;
      loop {
        interval.tick().await;
        match utils::download_database(true).await {
          Ok(_) => reload_database(),
          Err(err) => error!("Error downloading new database: {:?}", err),
        }
      }
    });
  }

  let host = env::var("HOST").unwrap_or("0.0.0.0".to_string());
  let port = env::var("PORT")
    .unwrap_or("3000".to_string())
    .parse::<u16>()
    .unwrap();

  HttpServer::new(move || {
    App::new()
      .service(metadata)
      .service(lookup)
      .wrap(
        middleware::DefaultHeaders::new().add(("server", format!("maxmind-geoip-api/{}", version))),
      )
      .wrap(middleware::Logger::new(
        env::var("ACCESS_LOG_FORMAT")
          .unwrap_or(String::from(
            r#"%{r}a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T"#,
          ))
          .as_str(),
      ))
  })
  .bind((host, port))?
  .run()
  .await
}
