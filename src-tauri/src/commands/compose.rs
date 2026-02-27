use bollard::container::ListContainersOptions;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter, State};

use crate::config::AppState;
use crate::error::AppError;
use crate::registry::{Stack, StacksRegistry};

#[derive(Debug, Serialize, Clone)]
pub struct StackItem {
    pub id: String,
    pub name: String,
    pub compose_file: String,
    pub missing: bool,
    pub services: Vec<ServiceItem>,
    pub status: StackStatus,
}

#[derive(Debug, Serialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StackStatus {
    AllRunning,
    Partial,
    Stopped,
    #[allow(dead_code)]
    Unknown,
}

#[derive(Debug, Serialize, Clone)]
pub struct ServiceItem {
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: Vec<String>,
}

/// Discover stacks from container labels + registered stacks
#[tauri::command]
pub async fn list_stacks(
    state: State<'_, AppState>,
    registry: State<'_, StacksRegistry>,
) -> Result<Vec<StackItem>, AppError> {
    let docker = state.get_docker().await?;

    // All containers to extract compose labels
    let containers = docker
        .list_containers(Some(ListContainersOptions::<String> {
            all: true,
            ..Default::default()
        }))
        .await?;

    // Group containers by compose project label
    let mut discovered: HashMap<String, Vec<_>> = HashMap::new();
    for c in &containers {
        if let Some(labels) = &c.labels {
            if let Some(project) = labels.get("com.docker.compose.project") {
                discovered
                    .entry(project.clone())
                    .or_default()
                    .push(c.clone());
            }
        }
    }

    // Registered stacks (user-added)
    let registered = registry.get_all().await;

    let mut result: Vec<StackItem> = Vec::new();

    // Start from registered stacks — they have a known compose file path
    for stack in &registered {
        let containers_for_project = discovered.remove(&stack.name);
        let services = containers_for_project
            .as_deref()
            .map(build_service_items)
            .unwrap_or_default();
        let status = compute_status(&services);
        result.push(StackItem {
            id: stack.id.clone(),
            name: stack.name.clone(),
            compose_file: stack.compose_file.clone(),
            missing: stack.missing,
            services,
            status,
        });
    }

    // Add auto-discovered stacks not already in registry
    for (project, cs) in discovered {
        let services = build_service_items(&cs);
        let status = compute_status(&services);
        result.push(StackItem {
            id: format!("auto-{}", project),
            name: project,
            compose_file: String::new(),
            missing: false,
            services,
            status,
        });
    }

    Ok(result)
}

fn build_service_items(containers: &[bollard::models::ContainerSummary]) -> Vec<ServiceItem> {
    containers
        .iter()
        .map(|c| {
            let name = c
                .labels
                .as_ref()
                .and_then(|l| l.get("com.docker.compose.service"))
                .cloned()
                .unwrap_or_else(|| {
                    c.names
                        .as_deref()
                        .and_then(|n| n.first())
                        .cloned()
                        .unwrap_or_default()
                        .trim_start_matches('/')
                        .to_string()
                });
            let image = c.image.clone().unwrap_or_default();
            let state = c.state.clone().unwrap_or_default();
            let status = c.status.clone().unwrap_or_default();
            let ports = c
                .ports
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter_map(|p| {
                    p.public_port.map(|pp| format!("{}:{}->{}/{}", p.ip.as_deref().unwrap_or("0.0.0.0"), pp, p.private_port, p.typ.as_ref().map(|t| format!("{t:?}").to_lowercase()).unwrap_or_else(|| "tcp".into())))
                })
                .collect();
            ServiceItem { name, image, state, status, ports }
        })
        .collect()
}

fn compute_status(services: &[ServiceItem]) -> StackStatus {
    if services.is_empty() {
        return StackStatus::Stopped;
    }
    let running = services.iter().filter(|s| s.state == "running").count();
    if running == services.len() {
        StackStatus::AllRunning
    } else if running == 0 {
        StackStatus::Stopped
    } else {
        StackStatus::Partial
    }
}

#[tauri::command]
pub async fn register_stack(
    name: String,
    compose_file: String,
    app: AppHandle,
    registry: State<'_, StacksRegistry>,
) -> Result<Stack, AppError> {
    // Validate the file exists
    if !std::path::Path::new(&compose_file).exists() {
        return Err(AppError::InvalidArgument(format!(
            "File not found: {compose_file}"
        )));
    }
    registry.add(&app, name, compose_file).await
}

#[tauri::command]
pub async fn remove_stack(
    id: String,
    app: AppHandle,
    registry: State<'_, StacksRegistry>,
) -> Result<(), AppError> {
    registry.remove(&app, &id).await
}

fn run_compose(
    state: &AppState,
    compose_file: Option<&str>,
    project: &str,
    args: &[&str],
    app: &AppHandle,
    event: &str,
) -> Result<(), AppError> {
    let program = state
        .compose_binary
        .as_program()
        .ok_or(AppError::ComposeNotFound)?;

    let mut cmd = Command::new(program);
    for a in state.compose_binary.base_args() {
        cmd.arg(a);
    }
    if let Some(file) = compose_file {
        cmd.args(["-f", file]);
    }
    cmd.args(["-p", project]);
    cmd.args(args);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(AppError::from)?;

    // Stream stdout
    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let app_clone = app.clone();
        let event_owned = event.to_string();
        std::thread::spawn(move || {
            for line in reader.lines().flatten() {
                let _ = app_clone.emit(&event_owned, &line);
            }
        });
    }

    // Stream stderr
    if let Some(stderr) = child.stderr.take() {
        let reader = BufReader::new(stderr);
        let app_clone = app.clone();
        let event_owned = event.to_string();
        std::thread::spawn(move || {
            for line in reader.lines().flatten() {
                let _ = app_clone.emit(&event_owned, &format!("[stderr] {line}"));
            }
        });
    }

    let exit = child.wait().map_err(AppError::from)?;
    if exit.success() {
        Ok(())
    } else {
        Err(AppError::ComposeError {
            code: exit.code().unwrap_or(-1),
            stderr: "see log stream".to_string(),
        })
    }
}

#[tauri::command]
pub async fn stack_up(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, StacksRegistry>,
) -> Result<(), AppError> {
    let stack = registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::StackNotFound(id.clone()))?;
    let state_ref: &AppState = &state;
    let event = format!("compose-log-{}", id);
    // run_compose blocks; spawn off main async thread
    let state_socket = state_ref.socket_path.clone();
    let binary = state_ref.compose_binary.clone();
    let name = stack.name.clone();
    let file_owned = stack.compose_file.clone();
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let tmp_state = crate::config::AppState::with_resolved(state_socket, binary);
        run_compose(
            &tmp_state,
            if file_owned.is_empty() { None } else { Some(&file_owned) },
            &name,
            &["up", "-d"],
            &app_clone,
            &event,
        )
    })
    .await
    .map_err(|e| AppError::Io(e.to_string()))??;
    Ok(())
}

#[tauri::command]
pub async fn stack_down(
    id: String,
    #[allow(unused_variables)]
    remove_volumes: Option<bool>,
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, StacksRegistry>,
) -> Result<(), AppError> {
    let remove_volumes = remove_volumes.unwrap_or(false);
    let stack = registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::StackNotFound(id.clone()))?;
    let binary = state.compose_binary.clone();
    let socket = state.socket_path.clone();
    let name = stack.name.clone();
    let file = stack.compose_file.clone();
    let event = format!("compose-log-{}", id);
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let tmp_state = crate::config::AppState::with_resolved(socket, binary);
        let mut extra_args: Vec<&str> = vec!["down"];
        if remove_volumes {
            extra_args.push("--volumes");
        }
        run_compose(
            &tmp_state,
            if file.is_empty() { None } else { Some(&file) },
            &name,
            &extra_args,
            &app_clone,
            &event,
        )
    })
    .await
    .map_err(|e| AppError::Io(e.to_string()))??;
    Ok(())
}

#[tauri::command]
pub async fn stack_restart(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, StacksRegistry>,
) -> Result<(), AppError> {
    let stack = registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::StackNotFound(id.clone()))?;
    let binary = state.compose_binary.clone();
    let socket = state.socket_path.clone();
    let name = stack.name.clone();
    let file = stack.compose_file.clone();
    let event = format!("compose-log-{}", id);
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let tmp_state = crate::config::AppState::with_resolved(socket, binary);
        run_compose(
            &tmp_state,
            if file.is_empty() { None } else { Some(&file) },
            &name,
            &["restart"],
            &app_clone,
            &event,
        )
    })
    .await
    .map_err(|e| AppError::Io(e.to_string()))??;
    Ok(())
}

#[tauri::command]
pub async fn stack_logs(
    id: String,
    service: Option<String>,
    tail: Option<u32>,
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, StacksRegistry>,
) -> Result<(), AppError> {
    let tail_str = tail.unwrap_or(200).to_string();
    let stack = registry
        .get_by_id(&id)
        .await
        .ok_or_else(|| AppError::StackNotFound(id.clone()))?;
    let binary = state.compose_binary.clone();
    let socket = state.socket_path.clone();
    let name = stack.name.clone();
    let file = stack.compose_file.clone();
    let event = format!("compose-log-{}", id);
    let app_clone = app.clone();
    tokio::task::spawn_blocking(move || {
        let tmp_state = crate::config::AppState::with_resolved(socket, binary);
        let mut args = vec!["logs", "-f", "--tail", tail_str.as_str()];
        if let Some(ref svc) = service {
            args.push(svc.as_str());
        }
        run_compose(
            &tmp_state,
            if file.is_empty() { None } else { Some(&file) },
            &name,
            &args,
            &app_clone,
            &event,
        )
    })
    .await
    .map_err(|e| AppError::Io(e.to_string()))??;
    Ok(())
}
