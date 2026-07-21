//! Typed error taxonomy with stable exit codes, categories, and the versioned JSON error contract.

use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    Generic = 1,
    Usage = 2,
    Config = 3,
    AwsAuth = 4,
    AwsThrottled = 5,
    NotFound = 6,
    Validation = 7,
    Timeout = 8,
}

impl From<ExitCode> for i32 {
    fn from(code: ExitCode) -> i32 {
        code as i32
    }
}

#[derive(Error, Debug)]
pub enum ShimesuError {
    #[error("Error: {0}")]
    Generic(String),

    #[error("Usage error: {0}")]
    Usage(String),

    #[error("Configuration error: {0}")]
    Config(String),

    #[error("AWS authentication failed: {0}")]
    AwsAuth(String),

    #[error("AWS request throttled: {0}")]
    AwsThrottled(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Validation error: {0}")]
    Validation(String),

    #[error("Operation timed out: {0}")]
    Timeout(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("TOML parse error: {0}")]
    TomlParse(#[from] toml::de::Error),
}

impl ShimesuError {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            ShimesuError::Generic(_) => ExitCode::Generic,
            ShimesuError::Usage(_) => ExitCode::Usage,
            ShimesuError::Config(_) => ExitCode::Config,
            ShimesuError::AwsAuth(_) => ExitCode::AwsAuth,
            ShimesuError::AwsThrottled(_) => ExitCode::AwsThrottled,
            ShimesuError::NotFound(_) => ExitCode::NotFound,
            ShimesuError::Validation(_) => ExitCode::Validation,
            ShimesuError::Timeout(_) => ExitCode::Timeout,
            ShimesuError::Io(_) => ExitCode::Generic,
            ShimesuError::Json(_) => ExitCode::Generic,
            ShimesuError::TomlParse(_) => ExitCode::Config,
        }
    }

    pub fn category(&self) -> &'static str {
        match self {
            ShimesuError::Generic(_) => "generic",
            ShimesuError::Usage(_) => "usage",
            ShimesuError::Config(_) => "config",
            ShimesuError::AwsAuth(_) => "aws_auth",
            ShimesuError::AwsThrottled(_) => "aws_throttled",
            ShimesuError::NotFound(_) => "not_found",
            ShimesuError::Validation(_) => "validation",
            ShimesuError::Timeout(_) => "timeout",
            ShimesuError::Io(_) => "io",
            ShimesuError::Json(_) => "json",
            ShimesuError::TomlParse(_) => "config",
        }
    }
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub schema_version: &'static str,
    pub error: bool,
    pub error_category: String,
    pub message: String,
    pub exit_code: i32,
}

impl ErrorResponse {
    pub fn from_error(err: &ShimesuError) -> Self {
        Self {
            schema_version: "1",
            error: true,
            error_category: err.category().to_string(),
            message: err.to_string(),
            exit_code: i32::from(err.exit_code()),
        }
    }
}

pub type Result<T> = std::result::Result<T, ShimesuError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exit_codes() {
        assert_eq!(i32::from(ExitCode::Success), 0);
        assert_eq!(i32::from(ExitCode::Generic), 1);
        assert_eq!(i32::from(ExitCode::Usage), 2);
        assert_eq!(i32::from(ExitCode::Config), 3);
        assert_eq!(i32::from(ExitCode::AwsAuth), 4);
        assert_eq!(i32::from(ExitCode::AwsThrottled), 5);
        assert_eq!(i32::from(ExitCode::NotFound), 6);
        assert_eq!(i32::from(ExitCode::Validation), 7);
        assert_eq!(i32::from(ExitCode::Timeout), 8);
    }

    #[test]
    fn test_error_exit_codes() {
        assert_eq!(
            ShimesuError::Generic("test".into()).exit_code(),
            ExitCode::Generic
        );
        assert_eq!(
            ShimesuError::Usage("test".into()).exit_code(),
            ExitCode::Usage
        );
        assert_eq!(
            ShimesuError::Config("test".into()).exit_code(),
            ExitCode::Config
        );
        assert_eq!(
            ShimesuError::AwsAuth("test".into()).exit_code(),
            ExitCode::AwsAuth
        );
        assert_eq!(
            ShimesuError::AwsThrottled("test".into()).exit_code(),
            ExitCode::AwsThrottled
        );
        assert_eq!(
            ShimesuError::NotFound("test".into()).exit_code(),
            ExitCode::NotFound
        );
        assert_eq!(
            ShimesuError::Validation("bad slug".into()).exit_code(),
            ExitCode::Validation
        );
        assert_eq!(
            ShimesuError::Timeout("request timed out".into()).exit_code(),
            ExitCode::Timeout
        );
    }

    #[test]
    fn test_error_categories() {
        assert_eq!(
            ShimesuError::Validation("test".into()).category(),
            "validation"
        );
        assert_eq!(ShimesuError::AwsAuth("test".into()).category(), "aws_auth");
        assert_eq!(
            ShimesuError::NotFound("test".into()).category(),
            "not_found"
        );
        assert_eq!(ShimesuError::Timeout("test".into()).category(), "timeout");
    }

    #[test]
    fn test_error_response_json() {
        let err = ShimesuError::Validation("bad slug".into());
        let response = ErrorResponse::from_error(&err);

        assert_eq!(response.schema_version, "1");
        assert!(response.error);
        assert_eq!(response.error_category, "validation");
        assert_eq!(response.exit_code, 7);
    }
}
