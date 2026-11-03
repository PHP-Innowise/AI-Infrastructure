use std::fmt;
#[derive(Debug)]
pub struct HarnessError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
}
pub type Result<T> = std::result::Result<T, HarnessError>;
impl HarnessError {
    pub fn invalid(message: impl Into<String>) -> Self {
        Self {
            status: 400,
            code: "invalid-request",
            message: message.into(),
        }
    }
    pub fn conflict(message: impl Into<String>) -> Self {
        Self {
            status: 409,
            code: "conflict",
            message: message.into(),
        }
    }
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            status: 403,
            code: "forbidden",
            message: message.into(),
        }
    }
    pub fn unavailable(message: impl Into<String>) -> Self {
        Self {
            status: 503,
            code: "unavailable",
            message: message.into(),
        }
    }
    pub fn missing(message: impl Into<String>) -> Self {
        Self {
            status: 404,
            code: "not-found",
            message: message.into(),
        }
    }
}
impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for HarnessError {}
impl From<std::io::Error> for HarnessError {
    fn from(_: std::io::Error) -> Self {
        Self::unavailable("Local filesystem or process operation failed.")
    }
}
impl From<rusqlite::Error> for HarnessError {
    fn from(_: rusqlite::Error) -> Self {
        Self::unavailable("Local session database operation failed.")
    }
}
impl From<serde_json::Error> for HarnessError {
    fn from(_: serde_json::Error) -> Self {
        Self::invalid("Invalid JSON request.")
    }
}
impl From<infra_core::RuntimeError> for HarnessError {
    fn from(value: infra_core::RuntimeError) -> Self {
        use infra_core::ErrorKind;
        let status = match value.kind {
            ErrorKind::InvalidInput => 400,
            ErrorKind::NotFound => 404,
            ErrorKind::Conflict => 409,
            ErrorKind::Unauthorized | ErrorKind::Policy => 403,
            _ => 503,
        };
        let code = match value.code.as_deref() {
            Some("cleanup-unconfirmed") => "cleanup-unconfirmed",
            _ => "runtime-refused",
        };
        Self {
            status,
            code,
            message: value.message,
        }
    }
}
