#![allow(clippy::needless_return)]

use actix_cors::Cors;
use actix_web::{App, HttpResponse, HttpServer, get, http::header, middleware, post, web};
use chrono::{TimeZone, Utc};
use log::{debug, error, info};
use maxminddb::MaxMindDbError;
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

use crate::types::Database;

pub mod types;
pub mod utils;

const VERSION: Option<&str> = option_env!("CARGO_PKG_VERSION");
const INITIAL_CHECK_INTERVAL: Duration = Duration::from_secs(30);
const UPDATE_CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const X_MAXMIND_NETWORK: header::HeaderName = header::HeaderName::from_static("x-maxmind-network");
const X_MAXMIND_BUILD_EPOCH: header::HeaderName = header::HeaderName::from_static("x-maxmind-build-epoch");

static DB_LOCK: OnceLock<RwLock<Option<Database>>> = OnceLock::new();

fn load_database() -> Result<Database, MaxMindDbError> {
  let db = Database::new(utils::database_path())?;

  if let Ok(build_epoch) = db.reader.metadata().build_epoch.try_into()
    && let Some(datetime) = Utc.timestamp_opt(build_epoch, 0).single()
  {
    info!(
      "Loaded a database of type {:?} ({:?}) dated {}",
      db.database_type,
      db.reader.metadata().database_type,
      datetime.format("%Y-%m-%d")
    );
  } else {
    info!("Loaded a database of type {:?} ({:?})", db.database_type, db.reader.metadata().database_type);
  }

  Ok(db)
}

fn reload_database() -> bool {
  match load_database() {
    Ok(new_db) => {
      let mut guard = DB_LOCK.wait().write().expect("error getting write-access to the db");
      *guard = Some(new_db);
      true
    }
    Err(err) => {
      error!("Error reloading database: {:?}", err);
      false
    }
  }
}

#[get("/metadata")]
async fn metadata() -> Result<HttpResponse, actix_web::error::Error> {
  let guard = DB_LOCK.wait().read().expect("error getting db");
  let db = match guard.as_ref() {
    Some(db) => db,
    None => {
      return Ok(HttpResponse::ServiceUnavailable().insert_header((header::RETRY_AFTER, INITIAL_CHECK_INTERVAL.as_secs())).finish());
    }
  };
  let metadata = db.reader.metadata();
  debug!("{:?}", metadata);

  return Ok(HttpResponse::Ok().insert_header((header::CONTENT_TYPE, "application/json")).body(json!(metadata).to_string()));
}

#[get("/{ip}")]
async fn lookup(addr: web::Path<IpAddr>) -> Result<HttpResponse, actix_web::error::Error> {
  let addr = addr.into_inner();
  debug!("addr: {}", addr);

  let guard = DB_LOCK.wait().read().expect("error getting db");
  let db = match guard.as_ref() {
    Some(db) => db,
    None => {
      return Ok(HttpResponse::ServiceUnavailable().insert_header((header::RETRY_AFTER, INITIAL_CHECK_INTERVAL.as_secs())).finish());
    }
  };
  let result = match db.reader.lookup(addr) {
    Ok(result) => result,
    Err(err) => {
      error!("Error looking up {}: {}", addr, err);
      return Ok(HttpResponse::InternalServerError().finish());
    }
  };
  let data = match db.database_type.decode(&result) {
    Ok(Some(data)) => data,
    Ok(None) => return Ok(HttpResponse::NotFound().finish()),
    Err(err) => {
      error!("Error decoding data for {}: {}", addr, err);
      return Ok(HttpResponse::InternalServerError().finish());
    }
  };
  debug!("data: {:?}", data);

  let mut response_builder = HttpResponse::Ok();
  if let Ok(network) = result.network() {
    response_builder.insert_header((X_MAXMIND_NETWORK, network.to_string()));
  }

  return Ok(
    response_builder
      .insert_header((header::CONTENT_TYPE, "application/json"))
      .insert_header((X_MAXMIND_BUILD_EPOCH, db.reader.metadata().build_epoch))
      .body(json!(data).to_string()),
  );
}

#[post("/lookup")]
async fn batch_lookup(body: web::Json<Vec<IpAddr>>) -> Result<HttpResponse, actix_web::error::Error> {
  let addrs = body.into_inner();

  let limit = utils::batch_limit();
  if addrs.len() > limit {
    return Ok(HttpResponse::PayloadTooLarge().body(format!("Maximum of {limit} IP addresses per request")));
  }

  let guard = DB_LOCK.wait().read().expect("error getting db");
  let db = match guard.as_ref() {
    Some(db) => db,
    None => {
      return Ok(HttpResponse::ServiceUnavailable().insert_header((header::RETRY_AFTER, INITIAL_CHECK_INTERVAL.as_secs())).finish());
    }
  };
  let mut results = serde_json::Map::new();

  for addr in addrs {
    let result = match db.reader.lookup(addr) {
      Ok(result) => result,
      Err(err) => {
        error!("Error looking up {}: {}", addr, err);
        return Ok(HttpResponse::InternalServerError().finish());
      }
    };
    match db.database_type.decode(&result) {
      Ok(Some(data)) => results.insert(addr.to_string(), json!(data)),
      Ok(None) => results.insert(addr.to_string(), serde_json::Value::Null),
      Err(err) => {
        error!("Error decoding data for {}: {}", addr, err);
        return Ok(HttpResponse::InternalServerError().finish());
      }
    };
  }

  return Ok(
    HttpResponse::Ok()
      .insert_header((header::CONTENT_TYPE, "application/json"))
      .insert_header((X_MAXMIND_BUILD_EPOCH, db.reader.metadata().build_epoch))
      .body(serde_json::Value::Object(results).to_string()),
  );
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

  let version = VERSION.unwrap_or("unknown");
  info!("version {}", version);

  // Send the process a SIGTERM to terminate the program
  tokio::spawn(async {
    let mut sigterm = signal(SignalKind::terminate()).expect("error listening for SIGTERM");
    sigterm.recv().await;
    debug!("Received SIGTERM");
    process::exit(0);
  });

  // Asynchronously download and load the database file while starting the HTTP server below
  // Requests that come in during this initialization will block on the database becoming ready
  tokio::spawn(async {
    let database_url_configured = env::var("MAXMIND_DB_URL").is_ok();
    let database_path = utils::database_path();

    // If MAXMIND_DB_URL is configured then try to download the database
    // Skip the check if the file was downloaded recently
    if database_url_configured
      && utils::old_stamp()
      && let Err(err) = utils::download_database().await
    {
      error!("Error downloading database: {}", err);
    }

    if database_path.is_file() {
      // Initialize the database
      DB_LOCK.get_or_init(|| {
        RwLock::new(
          load_database()
            .inspect_err(|err| {
              error!("Error initializing database: {:?}", err);
            })
            .ok(),
        )
      });
    } else {
      // Uh oh, we don't have a database.. requests will start failing
      DB_LOCK.get_or_init(|| RwLock::new(None));

      if !database_url_configured {
        // Exit with an error if there isn't a database file available on disk
        error!("Please configure MAXMIND_DB_URL or place a database file at {}", database_path.display());
        process::exit(1);
      }

      // Since we don't have a database, try to get the database every 30 seconds until we get a working database
      tokio::spawn(async {
        let mut interval = interval(INITIAL_CHECK_INTERVAL);
        interval.tick().await;
        loop {
          interval.tick().await;
          match utils::download_database().await {
            Ok(true) => {
              if reload_database() {
                break;
              }
            }
            Ok(false) => (),
            Err(err) => error!("Error downloading database: {}", err),
          }
        }
      });
    }

    if database_url_configured {
      // Send the process a SIGHUP to download a new database
      tokio::spawn(async {
        let mut sighup = signal(SignalKind::hangup()).expect("error listening for SIGHUP");
        while sighup.recv().await.is_some() {
          match utils::download_database().await {
            Ok(true) => {
              reload_database();
            }
            Ok(false) => (),
            Err(err) => error!("Error downloading new database: {}", err),
          }
        }
      });

      // Check for database updates every 24 hours
      tokio::spawn(async {
        let mut interval = interval(UPDATE_CHECK_INTERVAL);
        interval.tick().await;
        loop {
          interval.tick().await;
          match utils::download_database().await {
            Ok(true) => {
              reload_database();
            }
            Ok(false) => (),
            Err(err) => error!("Error downloading new database: {}", err),
          }
        }
      });
    } else {
      // If the program is running without MAXMIND_DB_URL then a SIGHUP simply re-opens the database from disk, which makes it possible to replace the database
      tokio::spawn(async {
        let mut sighup = signal(SignalKind::hangup()).expect("error listening for SIGHUP");
        while sighup.recv().await.is_some() {
          reload_database();
        }
      });
    }
  });

  let host = env::var("HOST").unwrap_or("::".to_string());
  let port = env::var("PORT").unwrap_or("3000".to_string()).parse::<u16>().unwrap();

  HttpServer::new(move || {
    let cors_allowed_origins = env::var("CORS_ALLOWED_ORIGINS");
    let mut cors = Cors::default();
    if let Ok(ref v) = cors_allowed_origins {
      cors = cors
        .allowed_methods(vec!["GET", "POST"])
        .expose_headers(vec![header::SERVER, X_MAXMIND_BUILD_EPOCH, X_MAXMIND_NETWORK])
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
      .wrap(middleware::Condition::new(cors_allowed_origins.is_ok(), cors))
      .wrap(middleware::DefaultHeaders::new().add((header::SERVER, format!("maxmind-geoip-api/{}", version))))
      .wrap(middleware::Logger::new(
        env::var("ACCESS_LOG_FORMAT").unwrap_or(String::from(r#"%{r}a "%r" %s %b "%{Origin}i" "%{User-Agent}i" %T"#)).as_str(),
      ))
  })
  .bind((host, port))?
  .run()
  .await
}
