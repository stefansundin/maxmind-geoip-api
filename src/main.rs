use actix_web::{get, middleware, web, App, HttpResponse, HttpServer};
use log::{debug, error};
use maxminddb::{geoip2, Metadata, Reader};
use serde::Serialize;
use serde_json::json;
use std::{collections::BTreeMap, env, net::IpAddr, process};
use tokio::signal::unix::{signal, SignalKind};
use tokio::time::{interval, Duration};

pub mod utils;

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
  let reader = Reader::open_mmap(utils::database_path()).map_err(|err| {
    error!("Error opening database: {}", err);
    actix_web::error::ErrorInternalServerError(err.to_string())
  })?;
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

  let reader = Reader::open_mmap(utils::database_path()).map_err(|err| {
    error!("Error opening database: {}", err);
    actix_web::error::ErrorInternalServerError(err.to_string())
  })?;

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

  // Send the process a SIGHUP to download a new database
  tokio::spawn(async {
    let mut sighup = signal(SignalKind::hangup()).expect("error listening for SIGHUP");
    while let Some(_) = sighup.recv().await {
      match utils::download_database(true).await {
        Ok(_) => {}
        Err(err) => error!("Error downloading new database: {:?}", err),
      }
    }
  });

  if let Err(err) = utils::download_database(false).await {
    error!("Error downloading database: {:?}", err);
    process::exit(1);
  }

  // Check for database updates every 24 hours
  if env::var("MAXMIND_DB_URL").is_ok() {
    tokio::spawn(async {
      let mut interval = interval(Duration::from_secs(24 * 60 * 60));
      loop {
        interval.tick().await;
        match utils::download_database(true).await {
          Ok(_) => {}
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

  HttpServer::new(|| {
    App::new()
      .service(metadata)
      .service(lookup)
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
