use core::fmt;
use maxminddb::MaxMindDbError;
use std::error::Error;

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
        write!(f, "error: {}", err)
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
        write!(f, "reqwest error: {}", err)
      }
      DatabaseDownloadError::UnexpectedResponseCode(status_code) => {
        write!(f, "unexpected response code: {}", status_code)
      }
      DatabaseDownloadError::ExtractDatabaseFileError(ref err) => {
        write!(
          f,
          "could not extract an .mmdb file from the archive: {}",
          err
        )
      }
      DatabaseDownloadError::IoError(ref err) => {
        write!(f, "i/o error: {}", err)
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
