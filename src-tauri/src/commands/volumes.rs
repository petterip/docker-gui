use bollard::volume::{CreateVolumeOptions, ListVolumesOptions, RemoveVolumeOptions};
use bollard::models::Volume;
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;

use crate::config::AppState;
use crate::error::AppError;

#[derive(Debug, Serialize, Clone)]
pub struct VolumeItem {
    pub name: String,
    pub driver: String,
    pub mount_point: String,
    pub created_at: Option<String>,
    pub labels: HashMap<String, String>,
    pub in_use: bool,
}

fn map_volume(v: Volume) -> VolumeItem {
    VolumeItem {
        name: v.name,
        driver: v.driver,
        mount_point: v.mountpoint,
        created_at: v.created_at.map(|d| d.to_string()),
        labels: v.labels,
        in_use: false, // will be set after container query
    }
}

#[tauri::command]
pub async fn list_volumes(state: State<'_, AppState>) -> Result<Vec<VolumeItem>, AppError> {
    let docker = state.get_docker().await?;

    // Fetch volumes and running containers in parallel
    let (volumes_resp, containers) = tokio::try_join!(
        docker.list_volumes(None::<ListVolumesOptions<String>>),
        docker.list_containers(Some(bollard::container::ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
    )
    .map_err(AppError::from)?;

    // Collect all volume names referenced by containers
    let used_volumes: std::collections::HashSet<String> = containers
        .iter()
        .flat_map(|c| c.mounts.as_deref().unwrap_or_default())
        .filter_map(|m| m.name.clone())
        .collect();

    let mut items: Vec<VolumeItem> = volumes_resp
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(map_volume)
        .collect();

    for item in &mut items {
        item.in_use = used_volumes.contains(&item.name);
    }

    Ok(items)
}

#[tauri::command]
pub async fn create_volume(
    name: Option<String>,
    state: State<'_, AppState>,
) -> Result<VolumeItem, AppError> {
    let docker = state.get_docker().await?;
    let options = CreateVolumeOptions {
        name: name.unwrap_or_default(),
        driver: "local".to_string(),
        ..Default::default()
    };
    let volume = docker.create_volume(options).await?;
    Ok(map_volume(volume))
}

#[tauri::command]
pub async fn remove_volume(
    name: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .remove_volume(&name, Some(RemoveVolumeOptions { force: false }))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn inspect_volume(
    name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let docker = state.get_docker().await?;
    let v = docker.inspect_volume(&name).await?;
    Ok(serde_json::to_value(v).map_err(AppError::from)?)
}
