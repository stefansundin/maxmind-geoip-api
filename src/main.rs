use actix_web::{get, middleware, web, App, HttpResponse, HttpServer};
use log::{debug, error};
use maxminddb::{geoip2, Reader};
use serde_json::json;
use std::{env, net::IpAddr, process};

pub mod utils;

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
      .body(json!(city).to_string()),
  );
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
  env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

  if let Err(err) = utils::download_database().await {
    error!("Error downloading database: {}", err);
    process::exit(1);
  }

  let host = env::var("HOST").unwrap_or("0.0.0.0".to_string());
  let port = env::var("PORT")
    .unwrap_or("3000".to_string())
    .parse::<u16>()
    .unwrap();

  HttpServer::new(|| {
    App::new().service(lookup).wrap(middleware::Logger::new(
      env::var("ACCESS_LOG_FORMAT")
        .unwrap_or(String::from(
          r#"%{r}a "%r" %s %b "%{Referer}i" "%{User-Agent}i" %T"#,
        ))
        .as_str(),
    ))
  })
  .bind((host.as_str(), port))?
  .run()
  .await
}
