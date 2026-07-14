use core::fmt;
use log::warn;
use maxminddb::{LookupResult, MaxMindDbError, Mmap, Reader, geoip2};
use std::{error::Error, path::Path};

#[derive(Debug)]
pub struct Database {
  pub reader: Reader<Mmap>,
  pub database_type: DatabaseType,
}

impl Database {
  pub fn new<P: AsRef<Path>>(database: P) -> Result<Self, MaxMindDbError> {
    let reader;
    unsafe {
      reader = Reader::open_mmap(database)?;
    }
    let database_type: DatabaseType = (&reader.metadata().database_type).into();
    Ok(Self { reader, database_type })
  }
}

#[derive(Debug)]
pub enum DatabaseType {
  City,
  Country,
  Enterprise,
  Isp,
  AnonymousIp,
  ConnectionType,
  Domain,
  Asn,
  DensityIncome,
}

impl From<&String> for DatabaseType {
  fn from(database_type: &String) -> Self {
    // The most comprehensive database type listing that I've found is here: https://github.com/oschwald/geoip2-golang/blob/09c8960066f4b46fc3c02a06f72daf602f4764df/reader.go#L148-L188
    // To avoid having a strict mapping here, at least for now, I have decided to just do a substring match.
    // It should be pretty resilient and will likely work well with third party databases too.
    let t = database_type.to_ascii_lowercase();
    if t.contains("city") {
      Self::City
    } else if t.contains("country") {
      Self::Country
    } else if t.contains("enterprise") {
      Self::Enterprise
    } else if t.contains("isp") {
      Self::Isp
    } else if t.contains("anonymous-ip") {
      Self::AnonymousIp
    } else if t.contains("connection-type") {
      Self::ConnectionType
    } else if t.contains("domain") {
      Self::Domain
    } else if t.contains("asn") {
      Self::Asn
    } else if t.contains("densityincome") {
      Self::DensityIncome
    } else {
      warn!("Unsupported database type, will attempt decoding as if it were a city database. Please report this issue if this is an official database type.");
      Self::City
    }
  }
}

impl DatabaseType {
  pub fn decode<'a, S>(&self, result: &'a LookupResult<'a, S>) -> Result<Option<LookupData<'a>>, MaxMindDbError>
  where
    S: AsRef<[u8]> + 'a,
  {
    match self {
      Self::City => match result.decode::<geoip2::City>() {
        Ok(Some(value)) => Ok(Some(LookupData::City(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::Country => match result.decode::<geoip2::Country>() {
        Ok(Some(value)) => Ok(Some(LookupData::Country(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::Enterprise => match result.decode::<geoip2::Enterprise>() {
        Ok(Some(value)) => Ok(Some(LookupData::Enterprise(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::Isp => match result.decode::<geoip2::Isp>() {
        Ok(Some(value)) => Ok(Some(LookupData::Isp(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::AnonymousIp => match result.decode::<geoip2::AnonymousIp>() {
        Ok(Some(value)) => Ok(Some(LookupData::AnonymousIp(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::ConnectionType => match result.decode::<geoip2::ConnectionType>() {
        Ok(Some(value)) => Ok(Some(LookupData::ConnectionType(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::Domain => match result.decode::<geoip2::Domain>() {
        Ok(Some(value)) => Ok(Some(LookupData::Domain(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::Asn => match result.decode::<geoip2::Asn>() {
        Ok(Some(value)) => Ok(Some(LookupData::Asn(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
      Self::DensityIncome => match result.decode::<geoip2::DensityIncome>() {
        Ok(Some(value)) => Ok(Some(LookupData::DensityIncome(value))),
        Ok(None) => Ok(None),
        Err(err) => Err(err),
      },
    }
  }
}

#[derive(serde::Serialize, Debug)]
#[serde(untagged)]
pub enum LookupData<'a> {
  City(geoip2::City<'a>),
  Country(geoip2::Country<'a>),
  Enterprise(geoip2::Enterprise<'a>),
  Isp(geoip2::Isp<'a>),
  AnonymousIp(geoip2::AnonymousIp),
  ConnectionType(geoip2::ConnectionType<'a>),
  Domain(geoip2::Domain<'a>),
  Asn(geoip2::Asn<'a>),
  DensityIncome(geoip2::DensityIncome),
}

pub enum ExtractDatabaseFileError {
  /// An mmdb file was not found in the archive.
  DatabaseFileNotFoundError,

  /// The downloaded database file could not be opened and was discarded.
  /// The database is likely invalid or corrupted.
  DatabaseInvalid(MaxMindDbError),

  /// Some other error.
  Error(Box<dyn Error>),
}

impl fmt::Display for ExtractDatabaseFileError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      ExtractDatabaseFileError::DatabaseFileNotFoundError => {
        write!(f, "could not find an .mmdb file in the archive")
      }
      ExtractDatabaseFileError::DatabaseInvalid(ref err) => {
        write!(f, "error opening newly downloaded database: {}", err)
      }
      ExtractDatabaseFileError::Error(ref err) => {
        write!(f, "error: {:?}", err)
      }
    }
  }
}

impl From<std::io::Error> for ExtractDatabaseFileError {
  fn from(err: std::io::Error) -> Self {
    Self::Error(Box::new(err))
  }
}

impl From<zip::result::ZipError> for ExtractDatabaseFileError {
  fn from(err: zip::result::ZipError) -> Self {
    Self::Error(Box::new(err))
  }
}

pub enum DatabaseDownloadError {
  /// The MAXMIND_DB_URL environment variable has not been configured.
  DatabaseUrlNotConfigured,

  /// Unexpected HTTP status code received.
  UnexpectedResponseCode(reqwest::StatusCode),

  /// Reqwest error.
  ReqwestError(reqwest::Error),

  /// An error was encountered when extracting the database file from the archive.
  ExtractDatabaseFileError(ExtractDatabaseFileError),

  /// An I/O error was encountered.
  IoError(std::io::Error),
}

impl fmt::Display for DatabaseDownloadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match *self {
      DatabaseDownloadError::DatabaseUrlNotConfigured => {
        write!(f, "MAXMIND_DB_URL is not set")
      }
      DatabaseDownloadError::ReqwestError(ref err) => {
        write!(f, "reqwest error: {:?}", err)
      }
      DatabaseDownloadError::UnexpectedResponseCode(status_code) => {
        write!(f, "unexpected response code: {}", status_code)
      }
      DatabaseDownloadError::ExtractDatabaseFileError(ref err) => {
        write!(f, "could not extract an .mmdb file from the archive: {}", err)
      }
      DatabaseDownloadError::IoError(ref err) => {
        write!(f, "i/o error: {:?}", err)
      }
    }
  }
}

impl From<reqwest::Error> for DatabaseDownloadError {
  fn from(err: reqwest::Error) -> Self {
    Self::ReqwestError(err)
  }
}

impl From<std::io::Error> for DatabaseDownloadError {
  fn from(err: std::io::Error) -> Self {
    Self::IoError(err)
  }
}
