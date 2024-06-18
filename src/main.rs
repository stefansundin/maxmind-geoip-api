use actix_cors::Cors;
use actix_web::middleware::Condition;
use actix_web::{get, middleware, web, App, HttpResponse, HttpServer};
use chrono::{TimeZone, Utc};
use log::info;
use log::{debug, error};
use maxminddb::{geoip2, Mmap, Reader};
use serde_json::json;
use std::sync::OnceLock;
use std::sync::RwLock;
use std::{env, net::IpAddr, process};
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

#[get("/metadata")]
async fn metadata() -> Result<HttpResponse, actix_web::error::Error> {
  let reader = reader_lock().read().expect("error getting reader");
  debug!("{:?}", reader.metadata);

  return Ok(
    HttpResponse::Ok()
      .append_header(("content-type", "application/json"))
      .body(json!(reader.metadata).to_string()),
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
    let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS");
    let mut cors = Cors::default();
    if let Ok(ref v) = cors_allowed_origins {
      cors = cors
        .allowed_methods(vec!["GET"])
        .expose_headers(vec!["server", "x-maxmind-build-epoch"])
        .max_age(3600);
      if v == "*" {
        cors = cors.allow_any_origin();
      } else {
        for origin in v.split(",") {
          cors = cors.allowed_origin(origin);
        }
      }
    }

    App::new()
      .service(metadata)
      .service(lookup)
      .wrap(Condition::new(cors_allowed_origins.is_ok(), cors))
      .wrap(
        middleware::DefaultHeaders::new().add(("server", format!("maxmind-geoip-api/{}", version))),
      )
      .wrap(middleware::Logger::new(
        env::var("ACCESS_LOG_FORMAT")
          .unwrap_or(String::from(
            r#"%{r}a "%r" %s %b "%{Origin}i" "%{User-Agent}i" %T"#,
          ))
          .as_str(),
      ))
  })
  .bind((host, port))?
  .run()
  .await
}
