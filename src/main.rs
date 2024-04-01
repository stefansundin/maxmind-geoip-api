use std::{env, net::IpAddr};

use actix_web::{get, middleware, web, App, HttpResponse, HttpServer};
use log::debug;
use maxminddb::{geoip2, Reader};
use serde_json::json;

#[get("/{ip}")]
async fn lookup(addr: web::Path<IpAddr>) -> HttpResponse {
  let addr = addr.into_inner();
  debug!("addr: {}", addr);

  let path = env::var("MAXMIND_DB_PATH").unwrap_or("GeoLite2-City.mmdb".to_string());
  let reader = Reader::open_mmap(path).expect("error getting reader");
  let result: Result<geoip2::City, _> = reader.lookup(addr);
  let city = match result {
    Ok(city) => city,
    Err(_) => return HttpResponse::NotFound().finish(),
  };
  debug!("city: {:?}", city);

  return HttpResponse::Ok()
    .append_header(("content-type", "application/json"))
    .body(json!(city).to_string());
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
  env_logger::init_from_env(env_logger::Env::default().default_filter_or("info"));

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
