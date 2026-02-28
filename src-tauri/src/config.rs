use bollard::Docker;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::error::AppError;

/// Resolved compose binary: either "docker" (v2 plugin) or "docker-compose" (v1)
#[derive(Debug, Clone)]
pub enum ComposeBinary {
    V2,           // invoke as: docker compose …
    V1(String),   // invoke as: docker-compose …
    NotFound,
}

impl ComposeBinary {
    pub fn as_program(&self) -> Option<&str> {
        match self {
            ComposeBinary::V2 => Some("docker"),
            ComposeBinary::V1(bin) => Some(bin.as_str()),
            ComposeBinary::NotFound => None,
        }
    }

    pub fn base_args(&self) -> Vec<String> {
        match self {
            ComposeBinary::V2 => vec!["compose".to_string()],
            ComposeBinary::V1(_) => vec![],
            ComposeBinary::NotFound => vec![],
        }
    }
}

pub struct AppState {
    pub docker: Arc<Mutex<Option<Docker>>>,
    pub socket_path: String,
    pub compose_binary: ComposeBinary,
}

impl AppState {
    /// Construct a lightweight state from already-resolved values (used in spawn_blocking contexts).
    pub fn with_resolved(socket_path: String, compose_binary: ComposeBinary) -> Self {
        let docker = connect_docker(&socket_path);
        AppState {
            docker: Arc::new(Mutex::new(docker)),
            socket_path,
            compose_binary,
        }
    }

    pub async fn new() -> Self {
        match resolve_socket_path() {
            Ok(socket_path) => {
                let docker = connect_docker(&socket_path);
                let compose_binary = resolve_compose_binary();
                AppState {
                    docker: Arc::new(Mutex::new(docker)),
                    socket_path,
                    compose_binary,
                }
            }
            Err(_) => {
                // Docker socket not found at startup — start in disconnected mode.
                // Commands will return SocketNotFound; the frontend will show a
                // "reconnect" prompt via check_connection.
                AppState {
                    docker: Arc::new(Mutex::new(None)),
                    socket_path: String::new(),
                    compose_binary: resolve_compose_binary(),
                }
            }
        }
    }

    pub async fn get_docker(&self) -> Result<Docker, AppError> {
        let guard = self.docker.lock().await;
        guard.clone().ok_or_else(|| {
            AppError::SocketNotFound(format!(
                "Could not connect to Docker at {}",
                self.socket_path
            ))
        })
    }
}

fn resolve_socket_path() -> Result<String, AppError> {
    // 1. DOCKER_HOST env var
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        return Ok(host);
    }

    // 2. Platform defaults
    #[cfg(target_os = "windows")]
    {
        return Ok("npipe:////./pipe/docker_engine".to_string());
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = dirs::home_dir() {
            let colima = home
                .join(".colima")
                .join("default")
                .join("docker.sock");
            if colima.exists() {
                return Ok(colima.to_string_lossy().to_string());
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // 3. Standard fallback (Linux / macOS fallback / WSL 2)
        let default = "/var/run/docker.sock";
        if std::path::Path::new(default).exists() {
            return Ok(default.to_string());
        }

        return Err(AppError::SocketNotFound(
            "No Docker socket found. Start Colima (macOS), Docker Engine (Linux/WSL), or Docker Desktop (Windows).".into(),
        ));
    }
}

fn connect_docker(socket_path: &str) -> Option<Docker> {
    #[cfg(target_os = "windows")]
    {
        return Docker::connect_with_named_pipe(socket_path, 30, bollard::API_DEFAULT_VERSION).ok();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let unix_path = socket_path.strip_prefix("unix://").unwrap_or(socket_path);
        return Docker::connect_with_unix(unix_path, 30, bollard::API_DEFAULT_VERSION).ok();
    }
}

fn resolve_compose_binary() -> ComposeBinary {
    // Prefer docker compose v2 plugin
    let v2 = std::process::Command::new("docker")
        .args(["compose", "version"])
        .output();
    if v2.map(|o| o.status.success()).unwrap_or(false) {
        return ComposeBinary::V2;
    }

    // Fall back to standalone docker-compose v1
    let v1 = std::process::Command::new("docker-compose")
        .arg("version")
        .output();
    if v1.map(|o| o.status.success()).unwrap_or(false) {
        return ComposeBinary::V1("docker-compose".to_string());
    }

    ComposeBinary::NotFound
}
