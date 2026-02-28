use serde::Serialize;
use tauri::State;

use crate::config::AppState;
use crate::error::AppError;

#[derive(Debug, Serialize)]
pub struct DockerInfo {
    pub server_version: String,
    pub api_version: String,
    pub socket_path: String,
    pub containers: i64,
    pub containers_running: i64,
    pub images: i64,
    pub os: String,
    pub arch: String,
}

#[tauri::command]
pub async fn get_docker_info(state: State<'_, AppState>) -> Result<DockerInfo, AppError> {
    let docker = state.get_docker().await?;
    let info = docker.info().await?;
    let version = docker.version().await?;
    let socket_path = state.get_socket_path().await;

    Ok(DockerInfo {
        server_version: info.server_version.unwrap_or_default(),
        api_version: version.api_version.unwrap_or_default(),
        socket_path,
        containers: info.containers.unwrap_or(0),
        containers_running: info.containers_running.unwrap_or(0),
        images: info.images.unwrap_or(0),
        os: info.operating_system.unwrap_or_default(),
        arch: info.architecture.unwrap_or_default(),
    })
}

#[tauri::command]
pub async fn check_connection(state: State<'_, AppState>) -> Result<bool, AppError> {
    if let Ok(docker) = state.get_docker().await {
        if docker.ping().await.is_ok() {
            return Ok(true);
        }
    }

    state.reconnect().await?;
    Ok(true)
}
