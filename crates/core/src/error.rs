//! Error type of the core crate. Messages are plain language: the UI shows them verbatim.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("Configuration problem: {0}")]
    Config(String),
    #[error("Rule pack problem: {0}")]
    RulePack(String),
    #[error("File problem: {0}")]
    Io(String),
    #[error("Report problem: {0}")]
    Report(String),
    #[error("BCF problem: {0}")]
    Bcf(String),
    #[error("Quantity takeoff problem: {0}")]
    Qto(String),
    #[error("{0}")]
    Other(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::Io(e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::Report(e.to_string())
    }
}
