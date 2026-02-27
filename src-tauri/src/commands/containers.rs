use bollard::container::{
    InspectContainerOptions, ListContainersOptions, LogOutput, LogsOptions, RemoveContainerOptions,
    RestartContainerOptions, StartContainerOptions, StopContainerOptions,
};
use bollard::models::ContainerSummary;
use futures_util::StreamExt;
use serde::Serialize;
use std::collections::HashMap;
use tauri::{AppHandle, Emitter, State};

use crate::config::AppState;
use crate::error::AppError;

#[derive(Debug, Serialize, Clone)]
pub struct ContainerItem {
    pub id: String,
    pub name: String,
    pub image: String,
    pub status: String,
    pub state: String,
    pub ports: Vec<PortMapping>,
    pub created: i64,
    pub labels: HashMap<String, String>,
}

#[derive(Debug, Serialize, Clone)]
pub struct PortMapping {
    pub host_ip: String,
    pub host_port: String,
    pub container_port: String,
    pub protocol: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct LogLine {
    pub stream: String, // "stdout" | "stderr"
    pub text: String,
}

fn map_container(c: ContainerSummary) -> ContainerItem {
    let id = c.id.unwrap_or_default();
    let name = c
        .names
        .unwrap_or_default()
        .first()
        .cloned()
        .unwrap_or_default()
        .trim_start_matches('/')
        .to_string();
    let image = c.image.unwrap_or_default();
    let status = c.status.unwrap_or_default();
    let state = c.state.unwrap_or_default();
    let created = c.created.unwrap_or(0);
    let labels = c.labels.unwrap_or_default();

    let ports = c
        .ports
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let container_port = p.private_port.to_string();
            let protocol = p.typ.map(|t| format!("{t:?}").to_lowercase()).unwrap_or_else(|| "tcp".into());
            let host_port = p.public_port.map(|pp| pp.to_string()).unwrap_or_default();
            let host_ip = p.ip.unwrap_or_default();
            if host_port.is_empty() {
                None
            } else {
                Some(PortMapping { host_ip, host_port, container_port, protocol })
            }
        })
        .collect();

    ContainerItem { id, name, image, status, state, ports, created, labels }
}

#[tauri::command]
pub async fn list_containers(state: State<'_, AppState>) -> Result<Vec<ContainerItem>, AppError> {
    let docker = state.get_docker().await?;
    let filters = HashMap::new();
    // all containers (running + stopped)
    let options = ListContainersOptions::<String> {
        all: true,
        filters,
        ..Default::default()
    };
    let containers = docker.list_containers(Some(options)).await?;
    Ok(containers.into_iter().map(map_container).collect())
}

#[tauri::command]
pub async fn start_container(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .start_container(&id, None::<StartContainerOptions<String>>)
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn stop_container(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .stop_container(&id, Some(StopContainerOptions { t: 10 }))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn restart_container(
    id: String,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .restart_container(&id, Some(RestartContainerOptions { t: 10 }))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn remove_container(
    id: String,
    remove_volumes: bool,
    force: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .remove_container(
            &id,
            Some(RemoveContainerOptions {
                v: remove_volumes,
                force,
                link: false,
            }),
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn get_container_logs(
    id: String,
    tail: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    let tail_str = tail.unwrap_or(200).to_string();
    let options = LogsOptions::<String> {
        follow: true,
        stdout: true,
        stderr: true,
        tail: tail_str,
        ..Default::default()
    };

    let mut stream = docker.logs(&id, Some(options));
    let event_name = format!("container-log-{}", id);

    while let Some(result) = stream.next().await {
        match result {
            Ok(output) => {
                let line = match output {
                    LogOutput::StdOut { message } => LogLine {
                        stream: "stdout".into(),
                        text: String::from_utf8_lossy(&message).to_string(),
                    },
                    LogOutput::StdErr { message } => LogLine {
                        stream: "stderr".into(),
                        text: String::from_utf8_lossy(&message).to_string(),
                    },
                    LogOutput::Console { message } => LogLine {
                        stream: "stdout".into(),
                        text: String::from_utf8_lossy(&message).to_string(),
                    },
                    _ => continue,
                };
                let _ = app.emit(&event_name, &line);
            }
            Err(e) => {
                let _ = app.emit(&event_name, &LogLine { stream: "stderr".into(), text: e.to_string() });
                break;
            }
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn inspect_container(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let docker = state.get_docker().await?;
    let info = docker
        .inspect_container(&id, None::<InspectContainerOptions>)
        .await?;
    Ok(serde_json::to_value(info).map_err(AppError::from)?)
}
