use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ErrorKind {
    Io,
    InvalidInput,
    NotFound,
    Conflict,
    Unauthorized,
    Policy,
    Unavailable,
}

#[derive(Debug)]
pub struct RuntimeError {
    pub kind: ErrorKind,
    pub message: String,
    pub code: Option<String>,
    pub os_code: Option<i32>,
}

pub type Result<T> = std::result::Result<T, RuntimeError>;

impl RuntimeError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: None,
            os_code: None,
        }
    }
    pub fn coded(kind: ErrorKind, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            code: Some(code.into()),
            os_code: None,
        }
    }
    pub fn refusal_code(&self) -> &str {
        self.code
            .as_deref()
            .unwrap_or("memory-result-v1:unclassified")
    }
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::InvalidInput, message)
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Conflict, message)
    }
    pub fn policy(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Policy, message)
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::Unavailable, message)
    }
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RuntimeError {}
impl From<std::io::Error> for RuntimeError {
    fn from(error: std::io::Error) -> Self {
        let mut value = Self::new(
            if error.kind() == std::io::ErrorKind::NotFound {
                ErrorKind::NotFound
            } else {
                ErrorKind::Io
            },
            format!("Filesystem operation failed: {error}"),
        );
        value.os_code = error.raw_os_error();
        value
    }
}
impl From<serde_json::Error> for RuntimeError {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("Invalid JSON")
    }
}
impl From<rusqlite::Error> for RuntimeError {
    fn from(error: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(failure, _) = &error {
            let code = match failure.extended_code & 255 {
                11 => Some("sqlite-corrupt"),
                26 => Some("sqlite-not-a-database"),
                5 | 6 => Some("sqlite-busy"),
                8 => Some("sqlite-read-only"),
                _ => None,
            };
            if let Some(code) = code {
                return Self::coded(
                    ErrorKind::Unavailable,
                    code,
                    "Local SQLite operation failed",
                );
            }
        }
        let message = if error.to_string().to_ascii_lowercase().contains("fts5") {
            "SQLite FTS5 support is required"
        } else {
            "Local SQLite operation failed"
        };
        Self::unavailable(message)
    }
}
