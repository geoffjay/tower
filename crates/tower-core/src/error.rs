//! Error codes + envelope (D§7 conventions).

use serde::{Deserialize, Serialize};

/// Machine-readable error codes (D§7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorCode {
    #[serde(rename = "not_found")]
    NotFound,
    #[serde(rename = "conflict")]
    Conflict,
    #[serde(rename = "timeout")]
    Timeout,
    #[serde(rename = "driver")]
    Driver,
    #[serde(rename = "invalid")]
    Invalid,
    #[serde(rename = "unauthorized")]
    Unauthorized,
    #[serde(rename = "machine_offline")]
    MachineOffline,
}

impl ErrorCode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ErrorCode::NotFound => "not_found",
            ErrorCode::Conflict => "conflict",
            ErrorCode::Timeout => "timeout",
            ErrorCode::Driver => "driver",
            ErrorCode::Invalid => "invalid",
            ErrorCode::Unauthorized => "unauthorized",
            ErrorCode::MachineOffline => "machine_offline",
        }
    }

    /// HTTP status the code maps to.
    pub fn http_status(&self) -> u16 {
        match self {
            ErrorCode::NotFound => 404,
            ErrorCode::Conflict => 409,
            ErrorCode::Timeout => 408,
            ErrorCode::Driver => 500,
            ErrorCode::Invalid => 400,
            ErrorCode::Unauthorized => 401,
            ErrorCode::MachineOffline => 503,
        }
    }
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Wire error envelope (D§7).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TowerError {
    pub code: ErrorCode,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl std::fmt::Display for TowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for TowerError {}

impl TowerError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        TowerError {
            code,
            message: message.into(),
            detail: None,
        }
    }

    pub fn not_found(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::NotFound, msg)
    }

    pub fn conflict(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, msg)
    }

    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Invalid, msg)
    }

    pub fn driver(msg: impl Into<String>) -> Self {
        Self::new(ErrorCode::Driver, msg)
    }
}
