use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error, Serialize)]
#[serde(tag = "kind", content = "message")]
pub enum AppError {
    #[error("Docker socket not found: {0}")]
    SocketNotFound(String),

    #[error("Docker API error: {0}")]
    DockerApi(String),

    #[error("Permission denied on socket: {0}")]
    PermissionDenied(String),

    #[error("Compose CLI error (exit {code}): {stderr}")]
    ComposeError { code: i32, stderr: String },

    #[error("Compose CLI not found in PATH")]
    ComposeNotFound,

    #[error("Stack not found: {0}")]
    StackNotFound(String),

    #[error("Stack registry IO error: {0}")]
    RegistryError(String),

    #[error("IO error: {0}")]
    Io(String),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

impl From<bollard::errors::Error> for AppError {
    fn from(e: bollard::errors::Error) -> Self {
        let msg = e.to_string();
        if msg.contains("Permission denied") {
            AppError::PermissionDenied(msg)
        } else if msg.contains("No such file") || msg.contains("connect") {
            AppError::SocketNotFound(msg)
        } else {
            AppError::DockerApi(msg)
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind;
        match e.kind() {
            ErrorKind::PermissionDenied => AppError::PermissionDenied(e.to_string()),
            ErrorKind::NotFound => AppError::SocketNotFound(e.to_string()),
            _ => AppError::Io(e.to_string()),
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::RegistryError(e.to_string())
    }
}
