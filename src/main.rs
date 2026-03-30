#![allow(clippy::needless_return)]

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, get, middleware, post, web};
use chrono::{TimeZone, Utc};
use log::{debug, error, info};
use maxminddb::{Mmap, Reader, geoip2};
use serde_json::json;
use std::{
  env,
  net::IpAddr,
  process,
  sync::{OnceLock, RwLock},
};
use tokio::{
  signal::unix::{SignalKind, signal},
  time::{Duration, interval},
};

pub mod utils;

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

fn load_database() -> Reader<Mmap> {
  let reader;
  unsafe {
    reader = Reader::open_mmap(utils::database_path()).expect("error opening database");
  }
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
      .insert_header(("content-type", "application/json"))
      .body(json!(reader.metadata).to_string()),
  );
}

#[get("/{ip}")]
async fn lookup(addr: web::Path<IpAddr>) -> Result<HttpResponse, actix_web::error::Error> {
  let addr = addr.into_inner();
  debug!("addr: {}", addr);

  let reader = reader_lock().read().expect("error getting reader");
  let network;
  let city = match reader.lookup(addr) {
    Ok(result) => {
      network = result.network();
      match result.decode::<geoip2::City>() {
        Ok(Some(city)) => city,
        Ok(None) => return Ok(HttpResponse::NotFound().finish()),
        Err(err) => {
          error!("Error looking up {}: {}", addr, err);
          return Ok(HttpResponse::InternalServerError().finish());
        }
      }
    }
    Err(err) => {
      error!("Error looking up {}: {}", addr, err);
      return Ok(HttpResponse::InternalServerError().finish());
    }
  };
  debug!("city: {:?}", city);

  let mut response_builder = HttpResponse::Ok();
  if let Ok(network) = network {
    response_builder.insert_header(("x-maxmind-network", network.to_string()));
  }

  return Ok(
    response_builder
      .insert_header(("content-type", "application/json"))
      .insert_header(("x-maxmind-build-epoch", reader.metadata.build_epoch))
      .body(json!(city).to_string()),
  );
}

#[post("/lookup")]
async fn batch_lookup(
  body: web::Json<Vec<IpAddr>>,
) -> Result<HttpResponse, actix_web::error::Error> {
  let addrs = body.into_inner();

  let limit = *utils::batch_limit();
  if addrs.len() > limit {
    return Ok(
      HttpResponse::PayloadTooLarge().body(format!("Maximum of {limit} IP addresses per request")),
    );
  }

  let reader = reader_lock().read().expect("error getting reader");

  let mut results = serde_json::Map::new();
  for addr in addrs {
    match reader.lookup(addr) {
      Ok(result) => match result.decode::<geoip2::City>() {
        Ok(Some(city)) => results.insert(addr.to_string(), json!(city)),
        Ok(None) => results.insert(addr.to_string(), serde_json::Value::Null),
        Err(err) => {
          error!("Error looking up {}: {}", addr, err);
          return Ok(HttpResponse::InternalServerError().finish());
        }
      },
      Err(err) => {
        error!("Error looking up {}: {}", addr, err);
        return Ok(HttpResponse::InternalServerError().finish());
      }
    };
  }

  return Ok(
    HttpResponse::Ok()
      .insert_header(("content-type", "application/json"))
      .insert_header(("x-maxmind-build-epoch", reader.metadata.build_epoch))
      .body(serde_json::Value::Object(results).to_string()),
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
    while sighup.recv().await.is_some() {
      match utils::download_database(true).await {
        Ok(true) => reload_database(),
        Ok(false) => (),
        Err(err) => error!("Error downloading new database: {}", err),
      }
    }
  });

  // Send the process a SIGTERM to terminate the program
  tokio::spawn(async {
    let mut sigterm = signal(SignalKind::terminate()).expect("error listening for SIGTERM");
    sigterm.recv().await;
    debug!("Received SIGTERM");
    process::exit(0);
  });

  if let Err(err) = utils::download_database(false).await {
    error!("Error downloading database: {}", err);
    process::exit(1);
  }

  // Load the database
  reader_lock();

  // Check for database updates every 24 hours
  if env::var("MAXMIND_DB_URL").is_ok() {
    tokio::spawn(async {
      let mut interval = interval(UPDATE_CHECK_INTERVAL);
      interval.tick().await;
      loop {
        interval.tick().await;
        match utils::download_database(true).await {
          Ok(true) => reload_database(),
          Ok(false) => (),
          Err(err) => error!("Error downloading new database: {}", err),
        }
      }
    });
  }

  let host = env::var("HOST").unwrap_or("::".to_string());
  let port = env::var("PORT")
    .unwrap_or("3000".to_string())
    .parse::<u16>()
    .unwrap();

  HttpServer::new(move || {
    let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS");
    let mut cors = Cors::default();
    if let Ok(ref v) = cors_allowed_origins {
      cors = cors
        .allowed_methods(vec!["GET", "POST"])
        .expose_headers(vec!["server", "x-maxmind-build-epoch"])
        .max_age(3600);
      if v == "*" {
        cors = cors.allow_any_origin();
      } else {
        for origin in v.split(',') {
          cors = cors.allowed_origin(origin);
        }
      }
    }

    App::new()
      .service(metadata)
      .service(batch_lookup)
      .service(lookup)
      .wrap(middleware::Condition::new(
        cors_allowed_origins.is_ok(),
        cors,
      ))
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
