use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Code {
    InvalidInput,
    NotInitialized,
    NotFound,
    Conflict,
    RequestConflict,
    DatabaseBusy,
    Storage,
    CorruptData,
    UnsupportedVersion,
    Exhausted,
    DestinationExists,
    RecoveryRequired,
}

#[derive(Debug, Serialize, JsonSchema, thiserror::Error)]
#[error("{message}")]
pub struct Error {
    pub code: Code,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_revision: Option<u64>,
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            path: None,
            retryable: false,
            current_revision: None,
        }
    }

    pub fn input(path: &str, message: impl Into<String>) -> Self {
        let mut error = Self::new(Code::InvalidInput, message);
        error.path = Some(path.into());
        error
    }

    pub fn storage(error: impl std::fmt::Display) -> Self {
        Self::new(Code::Storage, error.to_string())
    }

    pub fn corrupt(error: impl std::fmt::Display) -> Self {
        Self::new(Code::CorruptData, error.to_string())
    }
}
