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
    pub socket_path: Arc<Mutex<String>>,
    pub preferred_endpoint: Arc<Mutex<Option<String>>>,
    pub compose_binary: ComposeBinary,
}

impl AppState {
    /// Construct a lightweight state from already-resolved values (used in spawn_blocking contexts).
    pub fn with_resolved(socket_path: String, compose_binary: ComposeBinary) -> Self {
        let docker = connect_docker(&socket_path);
        AppState {
            docker: Arc::new(Mutex::new(docker)),
            socket_path: Arc::new(Mutex::new(socket_path)),
            preferred_endpoint: Arc::new(Mutex::new(None)),
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
                    socket_path: Arc::new(Mutex::new(socket_path)),
                    preferred_endpoint: Arc::new(Mutex::new(None)),
                    compose_binary,
                }
            }
            Err(_) => {
                // Docker socket not found at startup — start in disconnected mode.
                // Commands will return SocketNotFound; the frontend will show a
                // "reconnect" prompt via check_connection.
                AppState {
                    docker: Arc::new(Mutex::new(None)),
                    socket_path: Arc::new(Mutex::new(String::new())),
                    preferred_endpoint: Arc::new(Mutex::new(None)),
                    compose_binary: resolve_compose_binary(),
                }
            }
        }
    }

    pub async fn get_docker(&self) -> Result<Docker, AppError> {
        let guard = self.docker.lock().await;
        let socket_path = self.socket_path.lock().await.clone();
        guard.clone().ok_or_else(|| {
            AppError::SocketNotFound(format!(
                "Could not connect to Docker at {}",
                socket_path
            ))
        })
    }

    pub async fn get_socket_path(&self) -> String {
        self.socket_path.lock().await.clone()
    }

    pub async fn reconnect(&self) -> Result<(), AppError> {
        if let Some(preferred) = self.preferred_endpoint.lock().await.clone() {
            if self.reconnect_with_endpoint(&preferred).await.is_ok() {
                return Ok(());
            }
        }

        let socket_path = resolve_socket_path()?;
        self.reconnect_with_endpoint(&socket_path).await
    }

    pub async fn reconnect_with_endpoint(&self, endpoint: &str) -> Result<(), AppError> {
        let docker = connect_docker(endpoint).ok_or_else(|| {
            AppError::SocketNotFound(format!("Could not connect to Docker at {}", endpoint))
        })?;

        docker.ping().await?;

        *self.docker.lock().await = Some(docker);
        *self.socket_path.lock().await = endpoint.to_string();
        Ok(())
    }

    pub async fn set_preferred_endpoint(&self, endpoint: Option<String>) {
        *self.preferred_endpoint.lock().await = endpoint;
    }
}

pub(crate) fn resolve_socket_path() -> Result<String, AppError> {
    // 1. DOCKER_HOST env var
    if let Ok(host) = std::env::var("DOCKER_HOST") {
        if !host.trim().is_empty() {
            return Ok(host);
        }
    }

    // 2. Platform defaults
    #[cfg(target_os = "windows")]
    {
        if let Some(ctx_host) = resolve_windows_context_host() {
            return Ok(ctx_host);
        }
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

pub(crate) fn connect_docker(socket_path: &str) -> Option<Docker> {
    #[cfg(target_os = "windows")]
    {
        if socket_path.starts_with("npipe://") || socket_path.starts_with("//./pipe/") {
            return Docker::connect_with_named_pipe(
                socket_path,
                30,
                bollard::API_DEFAULT_VERSION,
            )
            .ok();
        }
        return Docker::connect_with_named_pipe(socket_path, 30, bollard::API_DEFAULT_VERSION).ok();
    }

    #[cfg(not(target_os = "windows"))]
    {
        let unix_path = socket_path.strip_prefix("unix://").unwrap_or(socket_path);
        return Docker::connect_with_unix(unix_path, 30, bollard::API_DEFAULT_VERSION).ok();
    }
}

#[cfg(target_os = "windows")]
fn resolve_windows_context_host() -> Option<String> {
    let show = std::process::Command::new("docker")
        .args(["context", "show"])
        .output()
        .ok()?;
    if !show.status.success() {
        return None;
    }

    let context = String::from_utf8(show.stdout).ok()?.trim().to_string();
    if context.is_empty() {
        return None;
    }

    let inspect = std::process::Command::new("docker")
        .args([
            "context",
            "inspect",
            "--format",
            "{{.Endpoints.docker.Host}}",
            &context,
        ])
        .output()
        .ok()?;
    if !inspect.status.success() {
        return None;
    }

    let host = String::from_utf8(inspect.stdout).ok()?.trim().to_string();
    if host.is_empty() { None } else { Some(host) }
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
