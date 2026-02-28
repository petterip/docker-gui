#![allow(dead_code, unused_variables)]

#[cfg(target_os = "windows")]
use chrono::Utc;
use bollard::volume::{CreateVolumeOptions, RemoveVolumeOptions};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use tokio::time::{sleep, Duration};
use uuid::Uuid;

use crate::config::{connect_docker, resolve_socket_path, AppState};
use crate::engine::{
    EngineRegistry, HostEngineKind, Provider, ProvisioningRunStatus, ProvisioningStage,
    ProvisioningStageStatus, ProvisioningState, DEFAULT_WSL_DISTRO, MANAGED_WSL_RELAY_PIPE,
};
use crate::error::AppError;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineHealth {
    Ready,
    NeedsRepair,
    NotInstalled,
}

#[derive(Debug, Serialize)]
pub struct EngineProviderStatus {
    pub id: String,
    pub label: String,
    pub active: bool,
    pub health: EngineHealth,
    pub endpoint: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct EngineStatus {
    pub active_provider_id: Option<String>,
    pub providers: Vec<EngineProviderStatus>,
    pub resume_checkpoint: Option<String>,
    pub provisioning: Option<ProvisioningState>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionGuidance {
    pub connected: bool,
    pub title: String,
    pub message: String,
    pub failure_class: Option<String>,
    pub primary_action: String,
}

#[derive(Debug, Serialize)]
pub struct PrivilegedActionContract {
    pub version: String,
    pub transport: String,
    pub supported_actions: Vec<PrivilegedActionSpec>,
    pub execution_mode: String,
    pub helper_binary: String,
}

#[derive(Debug, Serialize)]
pub struct PrivilegedActionSpec {
    pub id: String,
    pub description: String,
    pub requires_elevation: bool,
}

#[derive(Debug, Clone, Copy)]
enum PrivilegedAction {
    WslPrereqEnable,
    WslDistroInstall,
    WslEngineInstall,
    WslRelayRegister,
    HostEngineDetect,
    HostCompatibilityValidate,
}

impl PrivilegedAction {
    fn id(self) -> &'static str {
        match self {
            PrivilegedAction::WslPrereqEnable => "wsl_prereq_enable",
            PrivilegedAction::WslDistroInstall => "wsl_distro_install",
            PrivilegedAction::WslEngineInstall => "wsl_engine_install",
            PrivilegedAction::WslRelayRegister => "wsl_relay_register",
            PrivilegedAction::HostEngineDetect => "host_engine_detect",
            PrivilegedAction::HostCompatibilityValidate => "host_compatibility_validate",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PrivilegedExecutionMode {
    HelperWithInProcessFallback,
}

impl PrivilegedExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            PrivilegedExecutionMode::HelperWithInProcessFallback => {
                "helper_with_in_process_fallback"
            }
        }
    }
}

#[derive(Debug, Serialize)]
struct PrivilegedActionResult {
    action: String,
    status: &'static str,
    details: serde_json::Value,
}

#[derive(Debug)]
enum HelperDispatch {
    Handled(PrivilegedActionResult),
    Fallback { reason: String },
}

#[derive(Debug, Deserialize)]
struct HelperActionResponse {
    status: String,
    details: Option<serde_json::Value>,
    failure_class: Option<String>,
    message: Option<String>,
    retriable: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallProviderRequest {
    WslEngine,
    HostEngine,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchProviderRequest {
    WslEngine,
    HostEngine,
    CustomHost,
}

#[derive(Debug, Deserialize)]
pub struct SetCustomHostRequest {
    pub endpoint: String,
}

#[tauri::command]
pub async fn get_engine_status(
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
) -> Result<EngineStatus, AppError> {
    let config = registry.get().await;
    let active_id = config.active_provider.as_ref().map(|p| p.id().to_string());

    let wsl_provider = provider_for_wsl(config.active_provider.as_ref());
    let host_provider = provider_for_host(config.active_provider.as_ref());

    let providers = vec![
        provider_status(&state, &wsl_provider, active_id.as_deref() == Some("wsl_engine")).await,
        provider_status(&state, &host_provider, active_id.as_deref() == Some("host_engine")).await,
    ];

    Ok(EngineStatus {
        active_provider_id: active_id,
        providers,
        resume_checkpoint: config.resume_checkpoint,
        provisioning: config.provisioning,
    })
}

#[tauri::command]
pub async fn install_engine_provider(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    provider: InstallProviderRequest,
    consent: bool,
) -> Result<EngineStatus, AppError> {
    let target = match provider {
        InstallProviderRequest::WslEngine => provider_for_wsl(None),
        InstallProviderRequest::HostEngine => provider_for_host(None),
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "install_engine_provider")?;
    start_provisioning_run(app, state, registry, target, None).await
}

#[tauri::command]
pub async fn switch_active_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    provider: SwitchProviderRequest,
) -> Result<EngineStatus, AppError> {
    let config = registry.get().await;
    let previous_active = config.active_provider.clone();

    let target = match provider {
        SwitchProviderRequest::WslEngine => provider_for_wsl(config.active_provider.as_ref()),
        SwitchProviderRequest::HostEngine => provider_for_host(config.active_provider.as_ref()),
        SwitchProviderRequest::CustomHost => config
            .active_provider
            .clone()
            .filter(|p| matches!(p, Provider::CustomHost { .. }))
            .ok_or_else(|| {
                AppError::InvalidArgument(
                    "Set a Custom Host endpoint first before switching.".to_string(),
                )
            })?,
    };

    registry
        .set_resume_checkpoint(&app, Some("provider_switch".into()))
        .await?;
    registry.set_active_provider(&app, target.clone()).await?;
    state.set_preferred_endpoint(Some(target.endpoint())).await;

    if let Err(e) = state.reconnect_with_endpoint(&target.endpoint()).await {
        if let Some(previous) = previous_active {
            registry.set_active_provider(&app, previous.clone()).await?;
            state.set_preferred_endpoint(Some(previous.endpoint())).await;
            let _ = state.reconnect_with_endpoint(&previous.endpoint()).await;
            emit_engine_event(
                &app,
                "provider_switch_rolled_back",
                serde_json::json!({
                    "target_provider": target.id(),
                    "restored_provider": previous.id(),
                    "error": e.to_string(),
                }),
            );
        }
        registry.set_resume_checkpoint(&app, None).await?;
        return Err(e);
    }

    registry.set_resume_checkpoint(&app, None).await?;
    emit_engine_event(
        &app,
        "provider_switched",
        serde_json::json!({ "active_provider": target.id() }),
    );
    get_engine_status(state, registry).await
}

#[tauri::command]
pub async fn repair_active_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    consent: bool,
) -> Result<bool, AppError> {
    #[cfg(not(target_os = "windows"))]
    let _ = &app;
    let config = registry.get().await;
    if let Some(provider) = config.active_provider {
        ensure_privileged_consent_for_provider(&app, &provider, consent, "repair_active_engine")?;
        match &provider {
            #[cfg(target_os = "windows")]
            Provider::WslEngine { distro, relay_pipe } => {
                ensure_wsl_relay_registration_and_health(&app, distro, relay_pipe)
                    .await
                    .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
            }
            Provider::HostEngine { endpoint, .. } | Provider::CustomHost { endpoint } => {
                validate_host_compatibility(endpoint)
                    .await
                    .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
            }
            _ => {}
        }
        state
            .set_preferred_endpoint(Some(provider.endpoint()))
            .await;
    }
    state.reconnect().await?;
    emit_engine_event(&app, "provider_repaired", serde_json::json!({}));
    Ok(true)
}

#[tauri::command]
pub async fn get_connection_guidance(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
) -> Result<ConnectionGuidance, AppError> {
    if let Ok(docker) = state.get_docker().await {
        if docker.ping().await.is_ok() {
            return Ok(ConnectionGuidance {
                connected: true,
                title: "Connected".to_string(),
                message: "Container engine is ready.".to_string(),
                failure_class: None,
                primary_action: "none".to_string(),
            });
        }
    }

    let active_provider = registry.get().await.active_provider;
    if let Some(provider) = active_provider {
        state.set_preferred_endpoint(Some(provider.endpoint())).await;
    }

    match state.reconnect().await {
        Ok(()) => {
            emit_engine_event(
                &app,
                "reconnect_succeeded",
                serde_json::json!({ "source": "get_connection_guidance" }),
            );
            Ok(ConnectionGuidance {
            connected: true,
            title: "Connected".to_string(),
            message: "Container engine is ready.".to_string(),
            failure_class: None,
            primary_action: "none".to_string(),
            })
        }
        Err(e) => {
            emit_engine_event(
                &app,
                "reconnect_failed",
                serde_json::json!({
                    "source": "get_connection_guidance",
                    "failure_class": failure_class(&e),
                    "message": e.to_string(),
                }),
            );
            Ok(ConnectionGuidance {
                connected: false,
                title: "Container engine setup needed".to_string(),
                message: guidance_message(&e),
                failure_class: Some(failure_class(&e).to_string()),
                primary_action: "fix_automatically".to_string(),
            })
        }
    }
}

#[tauri::command]
pub async fn set_custom_host_endpoint(
    app: AppHandle,
    registry: State<'_, EngineRegistry>,
    request: SetCustomHostRequest,
) -> Result<(), AppError> {
    if request.endpoint.trim().is_empty() {
        return Err(AppError::InvalidArgument(
            "Custom host endpoint must not be empty.".into(),
        ));
    }

    registry
        .set_active_provider(
            &app,
            Provider::CustomHost {
                endpoint: request.endpoint.trim().to_string(),
            },
        )
        .await
}

#[tauri::command]
pub async fn get_privileged_action_contract() -> Result<PrivilegedActionContract, AppError> {
    let execution_mode = PrivilegedExecutionMode::HelperWithInProcessFallback;
    let helper_binary = "docker-gui-provisioning-helper".to_string();
    let supported_actions = vec![
        PrivilegedActionSpec {
            id: PrivilegedAction::WslPrereqEnable.id().to_string(),
            description: "Enable and validate Windows WSL prerequisites.".to_string(),
            requires_elevation: true,
        },
        PrivilegedActionSpec {
            id: PrivilegedAction::WslDistroInstall.id().to_string(),
            description: "Provision or select a supported WSL distro for engine setup.".to_string(),
            requires_elevation: true,
        },
        PrivilegedActionSpec {
            id: PrivilegedAction::WslEngineInstall.id().to_string(),
            description: "Install and validate Docker Engine packages inside WSL.".to_string(),
            requires_elevation: true,
        },
        PrivilegedActionSpec {
            id: PrivilegedAction::WslRelayRegister.id().to_string(),
            description: "Register and validate the managed WSL relay endpoint.".to_string(),
            requires_elevation: true,
        },
        PrivilegedActionSpec {
            id: PrivilegedAction::HostEngineDetect.id().to_string(),
            description: "Detect a compatible Host Engine endpoint.".to_string(),
            requires_elevation: false,
        },
        PrivilegedActionSpec {
            id: PrivilegedAction::HostCompatibilityValidate.id().to_string(),
            description: "Validate Host Engine API and compose compatibility.".to_string(),
            requires_elevation: false,
        },
    ];
    Ok(PrivilegedActionContract {
        version: "1.0".to_string(),
        transport: "json_command_contract".to_string(),
        supported_actions,
        execution_mode: execution_mode.as_str().to_string(),
        helper_binary,
    })
}

#[tauri::command]
pub async fn remove_managed_engine(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
) -> Result<EngineStatus, AppError> {
    registry.clear_managed_wsl_engine(&app).await?;
    let config = registry.get().await;
    state
        .set_preferred_endpoint(config.active_provider.map(|p| p.endpoint()))
        .await;
    get_engine_status(state, registry).await
}

#[tauri::command]
pub async fn start_engine_provisioning(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    provider: InstallProviderRequest,
    consent: bool,
) -> Result<ProvisioningState, AppError> {
    let target = match provider {
        InstallProviderRequest::WslEngine => provider_for_wsl(None),
        InstallProviderRequest::HostEngine => provider_for_host(None),
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "start_engine_provisioning")?;
    let status = start_provisioning_run(app, state, registry, target, None).await?;
    status
        .provisioning
        .ok_or_else(|| AppError::RegistryError("Provisioning state missing after start.".into()))
}

#[tauri::command]
pub async fn retry_engine_provisioning(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    consent: bool,
) -> Result<ProvisioningState, AppError> {
    let config = registry.get().await;
    let provisioning = config
        .provisioning
        .ok_or_else(|| AppError::InvalidArgument("No provisioning run available to retry.".into()))?;
    let resume_checkpoint = config.resume_checkpoint.clone();
    let target = match provisioning.target_provider_id.as_str() {
        "wsl_engine" => provider_for_wsl(config.active_provider.as_ref()),
        "host_engine" => provider_for_host(config.active_provider.as_ref()),
        _ => {
            return Err(AppError::InvalidArgument(
                "Retry is only supported for WSL Engine or Host Engine.".into(),
            ))
        }
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "retry_engine_provisioning")?;

    let status = start_provisioning_run(app, state, registry, target, resume_checkpoint).await?;
    status
        .provisioning
        .ok_or_else(|| AppError::RegistryError("Provisioning state missing after retry.".into()))
}

#[tauri::command]
pub async fn resume_engine_provisioning_if_needed(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
) -> Result<Option<ProvisioningState>, AppError> {
    let config = registry.get().await;
    let resume_checkpoint = match config.resume_checkpoint.clone() {
        Some(c) if !c.trim().is_empty() => c,
        _ => return Ok(None),
    };

    if matches!(
        config.provisioning,
        Some(ProvisioningState {
            status: ProvisioningRunStatus::Running,
            ..
        })
    ) {
        return Ok(config.provisioning);
    }

    if matches!(
        config.provisioning,
        Some(ProvisioningState {
            status: ProvisioningRunStatus::Succeeded,
            ..
        })
    ) {
        registry.set_resume_checkpoint(&app, None).await?;
        return Ok(None);
    }

    let target = match config
        .provisioning
        .as_ref()
        .map(|p| p.target_provider_id.as_str())
    {
        Some("wsl_engine") => provider_for_wsl(config.active_provider.as_ref()),
        Some("host_engine") => provider_for_host(config.active_provider.as_ref()),
        Some(other) => {
            return Err(AppError::InvalidArgument(format!(
                "Unknown provisioning target for resume: {other}"
            )))
        }
        None => match config.active_provider.as_ref().map(|p| p.id()) {
            Some("wsl_engine") => provider_for_wsl(config.active_provider.as_ref()),
            Some("host_engine") => provider_for_host(config.active_provider.as_ref()),
            _ => return Ok(None),
        },
    };

    emit_engine_event(
        &app,
        "provisioning_resume_requested",
        serde_json::json!({
            "resume_checkpoint": resume_checkpoint,
            "target_provider": target.id(),
        }),
    );
    let status =
        start_provisioning_run(app.clone(), state, registry, target, Some(resume_checkpoint)).await?;
    Ok(status.provisioning)
}

async fn provider_status(
    state: &State<'_, AppState>,
    provider: &Provider,
    active: bool,
) -> EngineProviderStatus {
    let endpoint = provider.endpoint();
    let health = if can_ping(&endpoint).await {
        EngineHealth::Ready
    } else if active {
        let socket = state.get_socket_path().await;
        if socket.is_empty() {
            EngineHealth::NotInstalled
        } else {
            EngineHealth::NeedsRepair
        }
    } else {
        EngineHealth::NotInstalled
    };

    EngineProviderStatus {
        id: provider.id().to_string(),
        label: provider.label().to_string(),
        active,
        health,
        endpoint: Some(endpoint),
    }
}

fn provider_for_wsl(active: Option<&Provider>) -> Provider {
    if let Some(Provider::WslEngine { distro, relay_pipe }) = active {
        return Provider::WslEngine {
            distro: distro.clone(),
            relay_pipe: relay_pipe.clone(),
        };
    }

    let discovered_distro =
        discover_preferred_wsl_distro().unwrap_or_else(|| DEFAULT_WSL_DISTRO.to_string());

    Provider::WslEngine {
        distro: discovered_distro,
        relay_pipe: MANAGED_WSL_RELAY_PIPE.to_string(),
    }
}

fn provider_for_host(active: Option<&Provider>) -> Provider {
    if let Some(Provider::HostEngine { kind, endpoint }) = active {
        return Provider::HostEngine {
            kind: kind.clone(),
            endpoint: endpoint.clone(),
        };
    }

    let endpoint = resolve_socket_path().unwrap_or_else(|_| {
        #[cfg(target_os = "windows")]
        {
            "npipe:////./pipe/docker_engine".to_string()
        }
        #[cfg(not(target_os = "windows"))]
        {
            "/var/run/docker.sock".to_string()
        }
    });
    Provider::HostEngine {
        kind: HostEngineKind::ExistingCompatibleHost,
        endpoint,
    }
}

async fn can_ping(endpoint: &str) -> bool {
    if let Some(docker) = connect_docker(endpoint) {
        return docker.ping().await.is_ok();
    }
    false
}

fn failure_class(error: &AppError) -> &'static str {
    match error {
        AppError::SocketNotFound(_) => "prereq_missing",
        AppError::PermissionDenied(_) => "connectivity_failed",
        AppError::DockerApi(_) => "engine_start_failed",
        _ => "connectivity_failed",
    }
}

fn guidance_message(error: &AppError) -> String {
    match error {
        AppError::SocketNotFound(_) => {
            "We could not find a running container engine. Select Fix automatically to repair setup."
                .to_string()
        }
        AppError::PermissionDenied(_) => {
            "Connection was blocked by permissions. Select Fix automatically to repair setup."
                .to_string()
        }
        _ => "Connection failed. Select Fix automatically to repair setup.".to_string(),
    }
}

fn provider_requires_elevation(provider: &Provider) -> bool {
    matches!(provider, Provider::WslEngine { .. })
}

fn ensure_privileged_consent_for_provider(
    app: &AppHandle,
    provider: &Provider,
    consent: bool,
    source: &str,
) -> Result<(), AppError> {
    if !provider_requires_elevation(provider) {
        return Ok(());
    }

    if consent {
        emit_engine_event(
            app,
            "privileged_consent_granted",
            serde_json::json!({
                "provider_id": provider.id(),
                "source": source,
            }),
        );
        return Ok(());
    }

    emit_engine_event(
        app,
        "privileged_consent_missing",
        serde_json::json!({
            "provider_id": provider.id(),
            "source": source,
        }),
    );
    Err(AppError::InvalidArgument(
        "Administrator permission is required for automatic WSL engine setup. Select Fix automatically again and approve the permission prompt.".to_string(),
    ))
}

fn emit_engine_event(app: &AppHandle, event_type: &str, details: serde_json::Value) {
    let _ = append_engine_event(app, event_type, details);
}

fn append_engine_event(
    app: &AppHandle,
    event_type: &str,
    details: serde_json::Value,
) -> Result<(), AppError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    let logs_dir = data_dir.join("logs");
    std::fs::create_dir_all(&logs_dir).map_err(|e| AppError::RegistryError(e.to_string()))?;
    let log_path = logs_dir.join("engine-events.jsonl");

    let entry = serde_json::json!({
        "timestamp": chrono::Utc::now().to_rfc3339(),
        "event_type": event_type,
        "details": details,
    });
    let line = format!(
        "{}\n",
        serde_json::to_string(&entry).map_err(|e| AppError::RegistryError(e.to_string()))?
    );

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    file.write_all(line.as_bytes())
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    Ok(())
}

async fn start_provisioning_run(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    target: Provider,
    start_stage: Option<String>,
) -> Result<EngineStatus, AppError> {
    if matches!(
        registry.get().await.provisioning,
        Some(ProvisioningState {
            status: ProvisioningRunStatus::Running,
            ..
        })
    ) {
        return Err(AppError::InvalidArgument(
            "Provisioning is already running.".into(),
        ));
    }

    let target_provider_id = target.id().to_string();
    let stage_ids = provisioning_stage_ids(&target);
    let start_index = match start_stage.as_deref() {
        Some(id) => stage_index(id, &stage_ids).ok_or_else(|| {
            AppError::InvalidArgument(format!(
                "Unknown provisioning checkpoint: {id}. Retry from Settings."
            ))
        })?,
        None => 0,
    };
    let mut stages = provisioning_stages(&target);
    for stage in stages.iter_mut().take(start_index) {
        stage.status = ProvisioningStageStatus::Completed;
    }
    let run_id = Uuid::new_v4().to_string();
    registry
        .begin_provisioning(&app, run_id.clone(), target_provider_id.clone(), stages)
        .await?;
    emit_engine_event(
        &app,
        "provisioning_started",
        serde_json::json!({
            "run_id": run_id,
            "target_provider_id": target_provider_id,
            "start_stage": start_stage,
        }),
    );
    if let Some(first) = stage_ids.get(start_index) {
        registry
            .set_resume_checkpoint(&app, Some((*first).to_string()))
            .await?;
    }

    let endpoint = target.endpoint();
    state.set_preferred_endpoint(Some(endpoint)).await;

    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_provisioning(app_for_task, target, start_index).await;
    });

    get_engine_status(state, registry).await
}

async fn run_provisioning(app: AppHandle, target: Provider, start_index: usize) -> Result<(), AppError> {
    let stage_ids = provisioning_stage_ids(&target);
    for (idx, stage_id) in stage_ids.iter().enumerate().skip(start_index) {
        emit_engine_event(
            &app,
            "provisioning_stage_started",
            serde_json::json!({ "stage_id": stage_id, "target_provider_id": target.id() }),
        );
        update_stage(&app, stage_id, ProvisioningStageStatus::InProgress, None, None).await?;
        if let Err(failure) = execute_stage_with_backoff(&app, &target, stage_id).await {
            let failure_message = failure.message.clone();
            update_stage(
                &app,
                stage_id,
                ProvisioningStageStatus::Failed,
                Some(failure.class.to_string()),
                Some(failure_message.clone()),
            )
            .await?;
            let registry = app.state::<EngineRegistry>();
            registry.finish_provisioning(&app, ProvisioningRunStatus::Failed).await?;
            registry
                .set_resume_checkpoint(&app, Some(stage_id.to_string()))
                .await?;
            emit_engine_event(
                &app,
                "provisioning_stage_failed",
                serde_json::json!({
                    "stage_id": stage_id,
                    "target_provider_id": target.id(),
                    "failure_class": failure.class,
                    "message": failure_message,
                }),
            );
            emit_engine_event(
                &app,
                "provisioning_failed",
                serde_json::json!({
                    "target_provider_id": target.id(),
                    "failed_stage": stage_id,
                }),
            );
            return Ok(());
        }

        update_stage(&app, stage_id, ProvisioningStageStatus::Completed, None, None).await?;
        emit_engine_event(
            &app,
            "provisioning_stage_completed",
            serde_json::json!({ "stage_id": stage_id, "target_provider_id": target.id() }),
        );
        let registry = app.state::<EngineRegistry>();
        let next_checkpoint = stage_ids.get(idx + 1).map(|s| s.to_string());
        registry.set_resume_checkpoint(&app, next_checkpoint).await?;
    }

    let registry = app.state::<EngineRegistry>();
    registry.set_active_provider(&app, target).await?;
    registry.finish_provisioning(&app, ProvisioningRunStatus::Succeeded).await?;
    registry.set_resume_checkpoint(&app, None).await?;
    emit_engine_event(
        &app,
        "provisioning_succeeded",
        serde_json::json!({}),
    );
    Ok(())
}

#[derive(Debug)]
struct StageFailure {
    class: &'static str,
    message: String,
    retriable: bool,
}

async fn run_privileged_action(
    app: &AppHandle,
    action: PrivilegedAction,
    target: &Provider,
    stage_id: &str,
) -> Result<PrivilegedActionResult, StageFailure> {
    let action_id = action.id();
    let mut execution_mode = "helper";
    emit_engine_event(
        app,
        "privileged_action_started",
        serde_json::json!({
            "action": action_id,
            "stage_id": stage_id,
            "target_provider_id": target.id(),
            "execution_mode": execution_mode,
        }),
    );

    let execution_result = match try_execute_helper_action(action, target) {
        Ok(HelperDispatch::Handled(result)) => Ok(result),
        Ok(HelperDispatch::Fallback { reason }) => {
            execution_mode = "in_process_fallback";
            emit_engine_event(
                app,
                "privileged_action_helper_fallback",
                serde_json::json!({
                    "action": action_id,
                    "stage_id": stage_id,
                    "target_provider_id": target.id(),
                    "reason": reason,
                }),
            );
            execute_privileged_action(app, action, target).await
        }
        Err(failure) => Err(failure),
    };

    match execution_result {
        Ok(result) => {
            emit_engine_event(
                app,
                "privileged_action_completed",
                serde_json::json!({
                    "action": action_id,
                    "stage_id": stage_id,
                    "target_provider_id": target.id(),
                    "status": result.status,
                    "execution_mode": execution_mode,
                }),
            );
            Ok(result)
        }
        Err(failure) => {
            emit_engine_event(
                app,
                "privileged_action_failed",
                serde_json::json!({
                    "action": action_id,
                    "stage_id": stage_id,
                    "target_provider_id": target.id(),
                    "failure_class": failure.class,
                    "message": failure.message,
                    "execution_mode": execution_mode,
                }),
            );
            Err(failure)
        }
    }
}

fn try_execute_helper_action(
    action: PrivilegedAction,
    target: &Provider,
) -> Result<HelperDispatch, StageFailure> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (action, target);
        return Ok(HelperDispatch::Fallback {
            reason: "helper_transport_not_available_on_this_platform".to_string(),
        });
    }

    #[cfg(target_os = "windows")]
    {
        let target_json = serde_json::to_string(target).map_err(|e| StageFailure {
            class: "helper_failed",
            message: format!("Could not serialize helper action payload: {e}"),
            retriable: false,
        })?;

        let output = match std::process::Command::new("docker-gui-provisioning-helper")
            .args([
                "run-action",
                "--action",
                action.id(),
                "--target-provider",
                target.id(),
                "--target-json",
                &target_json,
            ])
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                if e.kind() == std::io::ErrorKind::NotFound {
                    return Ok(HelperDispatch::Fallback {
                        reason: "helper_binary_not_found".to_string(),
                    });
                }
                return Ok(HelperDispatch::Fallback {
                    reason: format!("helper_launch_failed: {e}"),
                });
            }
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.trim().is_empty() {
            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Ok(HelperDispatch::Fallback {
                    reason: format!("helper_no_response: {}", stderr.trim()),
                });
            }
            return Ok(HelperDispatch::Handled(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({ "source": "helper_empty_success" }),
            }));
        }

        let response: HelperActionResponse = serde_json::from_str(&stdout).map_err(|e| StageFailure {
            class: "helper_failed",
            message: format!("Invalid helper response payload: {e}"),
            retriable: false,
        })?;

        if response.status.eq_ignore_ascii_case("succeeded") {
            return Ok(HelperDispatch::Handled(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: response.details.unwrap_or_else(|| serde_json::json!({})),
            }));
        }

        let class = map_helper_failure_class(response.failure_class.as_deref());
        let message = response.message.unwrap_or_else(|| {
            "Privileged helper action failed. Select Fix automatically to retry.".to_string()
        });
        Err(StageFailure {
            class,
            message,
            retriable: response.retriable.unwrap_or(true),
        })
    }
}

fn map_helper_failure_class(class: Option<&str>) -> &'static str {
    match class.unwrap_or("").trim() {
        "prereq_missing" => "prereq_missing",
        "reboot_required" => "reboot_required",
        "distro_install_failed" => "distro_install_failed",
        "engine_install_failed" => "engine_install_failed",
        "engine_start_failed" => "engine_start_failed",
        "relay_failed" => "relay_failed",
        "connectivity_failed" => "connectivity_failed",
        "host_not_installed" => "host_not_installed",
        "host_compat_failed" => "host_compat_failed",
        "permission_denied" => "permission_denied",
        _ => "helper_failed",
    }
}

async fn execute_privileged_action(
    app: &AppHandle,
    action: PrivilegedAction,
    target: &Provider,
) -> Result<PrivilegedActionResult, StageFailure> {
    #[cfg(not(target_os = "windows"))]
    let _ = app;

    match action {
        PrivilegedAction::HostEngineDetect => {
            detect_host_provider(target).await?;
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
        PrivilegedAction::HostCompatibilityValidate => {
            validate_host_provider_compatibility(target).await?;
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
        PrivilegedAction::WslPrereqEnable => {
            #[cfg(target_os = "windows")]
            {
                ensure_windows_wsl_prerequisites()?;
            }
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
        PrivilegedAction::WslDistroInstall => {
            ensure_wsl_distro_ready(target)?;
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
        PrivilegedAction::WslEngineInstall => {
            #[cfg(not(target_os = "windows"))]
            let _ = target;
            #[cfg(target_os = "windows")]
            if let Provider::WslEngine { distro, .. } = target {
                let selected_distro = resolve_wsl_engine_distro(distro)?;
                run_wsl_engine_install_script(&selected_distro)?;
            }
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
        PrivilegedAction::WslRelayRegister => {
            #[cfg(not(target_os = "windows"))]
            let _ = target;
            #[cfg(target_os = "windows")]
            if let Provider::WslEngine { distro, relay_pipe } = target {
                if relay_pipe.trim().is_empty() {
                    return Err(StageFailure {
                        class: "relay_failed",
                        message: "Relay endpoint is missing. Select Fix automatically to retry."
                            .to_string(),
                        retriable: false,
                    });
                }
                let selected_distro = resolve_wsl_engine_distro(distro)?;
                verify_wsl_engine_socket_ready(&selected_distro)?;
                ensure_wsl_relay_registration_and_health(app, &selected_distro, relay_pipe).await?;
            }
            Ok(PrivilegedActionResult {
                action: action.id().to_string(),
                status: "succeeded",
                details: serde_json::json!({}),
            })
        }
    }
}

#[cfg(target_os = "windows")]
#[derive(Debug, Serialize, Deserialize)]
struct WslRelayState {
    provider: String,
    distro: String,
    relay_pipe: String,
    status: String,
    last_checked_at: String,
    last_error: Option<String>,
}

fn provisioning_stage_ids(target: &Provider) -> Vec<&'static str> {
    provisioning_stage_specs(target)
        .iter()
        .map(|(id, _)| *id)
        .collect()
}

fn stage_index(stage_id: &str, stage_ids: &[&str]) -> Option<usize> {
    stage_ids.iter().position(|id| *id == stage_id)
}

async fn update_stage(
    app: &AppHandle,
    stage_id: &str,
    status: ProvisioningStageStatus,
    failure_class: Option<String>,
    message: Option<String>,
) -> Result<(), AppError> {
    let registry = app.state::<EngineRegistry>();
    registry
        .mark_stage(app, stage_id, status, failure_class, message)
        .await
}

fn provisioning_stages(target: &Provider) -> Vec<ProvisioningStage> {
    provisioning_stage_specs(target)
        .iter()
        .map(|(id, label)| ProvisioningStage {
            id: (*id).to_string(),
            label: (*label).to_string(),
            status: ProvisioningStageStatus::Pending,
            failure_class: None,
            message: None,
        })
        .collect()
}

fn provisioning_stage_specs(target: &Provider) -> &'static [(&'static str, &'static str)] {
    const WSL_STAGE_SPECS: [(&str, &str); 5] = [
        ("before_windows_features", "Checking prerequisites"),
        ("before_distro", "Preparing WSL distro"),
        ("before_engine_install", "Installing engine packages"),
        ("before_relay_registration", "Registering relay endpoint"),
        ("health_check", "Running health checks"),
    ];

    const HOST_STAGE_SPECS: [(&str, &str); 4] = [
        ("detect_host_provider", "Detecting host engine"),
        ("validate_host_endpoint", "Validating host endpoint"),
        ("validate_host_compatibility", "Checking host compatibility"),
        ("health_check", "Running health checks"),
    ];

    match target {
        Provider::WslEngine { .. } => &WSL_STAGE_SPECS,
        Provider::HostEngine { .. } | Provider::CustomHost { .. } => &HOST_STAGE_SPECS,
    }
}

async fn execute_stage_with_backoff(
    app: &AppHandle,
    target: &Provider,
    stage_id: &str,
) -> Result<(), StageFailure> {
    let mut wait = Duration::from_millis(500);
    let max_attempts = 3;
    for attempt in 1..=max_attempts {
        match execute_stage(app, target, stage_id).await {
            Ok(()) => return Ok(()),
            Err(failure) => {
                if !failure.retriable || attempt == max_attempts {
                    return Err(failure);
                }
                sleep(wait).await;
                wait *= 2;
            }
        }
    }
    Err(StageFailure {
        class: "connectivity_failed",
        message: "Provisioning retry exhausted. Select Fix automatically to retry.".to_string(),
        retriable: false,
    })
}

async fn execute_stage(app: &AppHandle, target: &Provider, stage_id: &str) -> Result<(), StageFailure> {
    #[cfg(not(target_os = "windows"))]
    let _ = app;

    match stage_id {
        "detect_host_provider" => {
            let _ = run_privileged_action(
                app,
                PrivilegedAction::HostEngineDetect,
                target,
                stage_id,
            )
            .await?;
            Ok(())
        }
        "validate_host_endpoint" => match target {
            Provider::HostEngine { endpoint, .. } | Provider::CustomHost { endpoint } => {
                if endpoint.trim().is_empty() {
                    Err(StageFailure {
                        class: "connectivity_failed",
                        message: "Host endpoint is missing. Select Fix automatically to retry."
                            .to_string(),
                        retriable: false,
                    })
                } else {
                    Ok(())
                }
            }
            _ => Err(StageFailure {
                class: "connectivity_failed",
                message: "Host endpoint validation is not available for this provider selection."
                    .to_string(),
                retriable: false,
            }),
        },
        "validate_host_compatibility" => {
            let _ = run_privileged_action(
                app,
                PrivilegedAction::HostCompatibilityValidate,
                target,
                stage_id,
            )
            .await?;
            Ok(())
        }
        "before_windows_features" => {
            let _ = run_privileged_action(
                app,
                PrivilegedAction::WslPrereqEnable,
                target,
                stage_id,
            )
            .await?;
            Ok(())
        }
        "before_distro" => match target {
            Provider::WslEngine { distro, .. } if distro.trim().is_empty() => {
                Err(StageFailure {
                    class: "prereq_missing",
                    message:
                        "Managed WSL distro is not configured. Select Fix automatically to retry."
                            .to_string(),
                    retriable: false,
                })
            }
            Provider::WslEngine { .. } => {
                let _ = run_privileged_action(
                    app,
                    PrivilegedAction::WslDistroInstall,
                    target,
                    stage_id,
                )
                .await?;
                Ok(())
            }
            _ => Ok(()),
        },
        "before_engine_install" => match target {
            Provider::WslEngine { .. } => {
                let _ = run_privileged_action(
                    app,
                    PrivilegedAction::WslEngineInstall,
                    target,
                    stage_id,
                )
                .await?;
                Ok(())
            }
            _ => Ok(()),
        },
        "before_relay_registration" => match target {
            Provider::WslEngine { .. } => {
                let _ = run_privileged_action(
                    app,
                    PrivilegedAction::WslRelayRegister,
                    target,
                    stage_id,
                )
                .await?;
                Ok(())
            }
            _ => Ok(()),
        },
        "health_check" => {
            match target {
                Provider::WslEngine {
                    distro,
                    relay_pipe,
                } => {
                    #[cfg(not(target_os = "windows"))]
                    let _ = (distro, relay_pipe);
                    #[cfg(target_os = "windows")]
                    {
                        let selected_distro = resolve_wsl_engine_distro(distro)?;
                        if can_ping(relay_pipe).await {
                            update_wsl_relay_state(
                                app,
                                &selected_distro,
                                relay_pipe,
                                "running",
                                None,
                            )?;
                            return Ok(());
                        }

                        if let Some(fallback) = windows_fallback_endpoint() {
                            if can_ping(&fallback).await {
                                update_wsl_relay_state(
                                    app,
                                    &selected_distro,
                                    relay_pipe,
                                    "degraded",
                                    Some(format!(
                                        "Managed relay unavailable, fallback endpoint in use: {fallback}"
                                    )),
                                )?;
                                return Ok(());
                            }
                        }

                        return Err(StageFailure {
                            class: "connectivity_failed",
                            message: "Engine health check failed. Select Fix automatically to retry."
                                .to_string(),
                            retriable: true,
                        });
                    }
                    #[cfg(not(target_os = "windows"))]
                    {
                        Ok(())
                    }
                }
                _ => {
                    let endpoint = target.endpoint();
                    if can_ping(&endpoint).await {
                        Ok(())
                    } else {
                        Err(StageFailure {
                            class: "connectivity_failed",
                            message: "Engine health check failed. Select Fix automatically to retry."
                                .to_string(),
                            retriable: true,
                        })
                    }
                }
            }
        }
        _ => Ok(()),
    }
}

fn discover_preferred_wsl_distro() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        let distros = list_wsl_distros().ok()?;
        if distros
            .iter()
            .any(|name| name.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO))
        {
            return Some(DEFAULT_WSL_DISTRO.to_string());
        }
        return distros
            .into_iter()
            .find(|name| is_supported_ubuntu_distro(name));
    }

    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

async fn detect_host_provider(target: &Provider) -> Result<(), StageFailure> {
    match target {
        Provider::HostEngine { kind, endpoint } => match kind {
            HostEngineKind::RancherDesktopMoby => Err(StageFailure {
                class: "host_policy_blocked",
                message: "Rancher Desktop UI install is disabled by policy. Use an existing compatible Host Engine or WSL Engine."
                    .to_string(),
                retriable: false,
            }),
            HostEngineKind::ExistingCompatibleHost => {
                if can_ping(endpoint).await {
                    Ok(())
                } else {
                    Err(StageFailure {
                        class: "host_not_installed",
                        message: "No compatible Host Engine was detected. Install or start a compatible host provider, or use WSL Engine."
                            .to_string(),
                        retriable: false,
                    })
                }
            }
        },
        Provider::CustomHost { endpoint } => {
            if can_ping(endpoint).await {
                Ok(())
            } else {
                Err(StageFailure {
                    class: "host_not_installed",
                    message: "No compatible Host Engine was detected. Install or start a compatible host provider, or use WSL Engine."
                        .to_string(),
                    retriable: false,
                })
            }
        }
        _ => Err(StageFailure {
            class: "prereq_missing",
            message: "Host engine detection is not available for this provider selection."
                .to_string(),
            retriable: false,
        }),
    }
}

async fn validate_host_provider_compatibility(target: &Provider) -> Result<(), StageFailure> {
    match target {
        Provider::HostEngine { endpoint, .. } | Provider::CustomHost { endpoint } => {
            validate_host_compatibility(endpoint).await
        }
        _ => Err(StageFailure {
            class: "host_compat_failed",
            message: "Host compatibility checks are not available for this provider selection."
                .to_string(),
            retriable: false,
        }),
    }
}

fn ensure_wsl_distro_ready(target: &Provider) -> Result<(), StageFailure> {
    #[cfg(not(target_os = "windows"))]
    let _ = target;
    #[cfg(target_os = "windows")]
    if let Provider::WslEngine { distro, .. } = target {
        let installed = list_wsl_distros()?;
        let selected_exists = installed
            .iter()
            .any(|name| name.eq_ignore_ascii_case(distro));
        if selected_exists {
            return Ok(());
        }

        if installed.iter().any(|name| is_supported_ubuntu_distro(name)) {
            return Ok(());
        }

        install_supported_wsl_distro()?;

        let installed_after = list_wsl_distros()?;
        if installed_after
            .iter()
            .any(|name| is_supported_ubuntu_distro(name))
        {
            return Ok(());
        }

        return Err(StageFailure {
            class: "prereq_missing",
            message: "WSL distro installation did not complete. Restart Windows and select Fix automatically to resume setup.".to_string(),
            retriable: false,
        });
    }
    Ok(())
}

#[cfg(target_os = "windows")]
fn list_wsl_distros() -> Result<Vec<String>, StageFailure> {
    let output = std::process::Command::new("wsl")
        .args(["-l", "-q"])
        .output()
        .map_err(|e| StageFailure {
            class: "prereq_missing",
            message: format!("Unable to query WSL distros: {e}. Select Fix automatically to retry."),
            retriable: false,
        })?;

    if !output.status.success() {
        return Err(StageFailure {
            class: "prereq_missing",
            message: "Unable to query WSL distros. Select Fix automatically to retry.".to_string(),
            retriable: false,
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let distros = stdout
        .lines()
        .map(|line| line.trim().trim_start_matches('*').trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect::<Vec<_>>();

    Ok(distros)
}

#[cfg(target_os = "windows")]
fn ensure_windows_wsl_prerequisites() -> Result<(), StageFailure> {
    if wsl_status_ok() {
        return Ok(());
    }

    let _ = run_windows_command(
        "wsl",
        &["--install", "--no-distribution"],
        "Failed to run WSL prerequisite installer.",
    );
    let _ = run_windows_command(
        "dism.exe",
        &[
            "/online",
            "/enable-feature",
            "/featurename:Microsoft-Windows-Subsystem-Linux",
            "/all",
            "/norestart",
        ],
        "Failed to enable Windows Subsystem for Linux feature.",
    );
    let _ = run_windows_command(
        "dism.exe",
        &[
            "/online",
            "/enable-feature",
            "/featurename:VirtualMachinePlatform",
            "/all",
            "/norestart",
        ],
        "Failed to enable Virtual Machine Platform feature.",
    );

    if wsl_status_ok() {
        return Ok(());
    }

    Err(StageFailure {
        class: "prereq_missing",
        message: "WSL prerequisites are still missing. Select Fix automatically to retry."
            .to_string(),
        retriable: false,
    })
}

#[cfg(target_os = "windows")]
fn wsl_status_ok() -> bool {
    std::process::Command::new("wsl")
        .arg("--status")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn run_windows_command(cmd: &str, args: &[&str], fail_prefix: &str) -> Result<(), StageFailure> {
    let output = std::process::Command::new(cmd)
        .args(args)
        .output()
        .map_err(|e| StageFailure {
            class: "prereq_missing",
            message: format!("{fail_prefix} {e}."),
            retriable: false,
        })?;
    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();

    if combined.contains("restart")
        || combined.contains("reboot")
        || combined.contains("0x80370102")
        || combined.contains("wsl must be updated")
    {
        return Err(StageFailure {
            class: "reboot_required",
            message: "Windows restart is required to finish prerequisite setup. Restart your device, then select Fix automatically."
                .to_string(),
            retriable: false,
        });
    }
    if combined.contains("access is denied")
        || combined.contains("0x80070005")
        || combined.contains("administrator")
        || combined.contains("elevation")
    {
        return Err(StageFailure {
            class: "permission_denied",
            message: "Administrator permissions are required for prerequisite setup. Re-run setup with admin rights, then select Fix automatically."
                .to_string(),
            retriable: false,
        });
    }
    Err(StageFailure {
        class: "prereq_missing",
        message: format!("{fail_prefix} Select Fix automatically to retry."),
        retriable: true,
    })
}

#[cfg(target_os = "windows")]
fn resolve_wsl_engine_distro(preferred: &str) -> Result<String, StageFailure> {
    let distros = list_wsl_distros()?;
    if distros
        .iter()
        .any(|name| name.eq_ignore_ascii_case(preferred))
    {
        return Ok(preferred.to_string());
    }

    if distros
        .iter()
        .any(|name| name.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO))
    {
        return Ok(DEFAULT_WSL_DISTRO.to_string());
    }

    if let Some(ubuntu) = distros
        .iter()
        .find(|name| is_supported_ubuntu_distro(name))
        .cloned()
    {
        return Ok(ubuntu);
    }

    Err(StageFailure {
        class: "prereq_missing",
        message: "No supported WSL distro is available for engine installation. Select Fix automatically to retry.".to_string(),
        retriable: false,
    })
}

#[cfg(target_os = "windows")]
fn run_wsl_engine_install_script(distro: &str) -> Result<(), StageFailure> {
    let script = r#"
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive

if ! command -v apt-get >/dev/null 2>&1; then
  echo "apt-get is required for automatic engine setup"
  exit 21
fi

apt-get update -y
apt-get install -y ca-certificates curl gnupg lsb-release
apt-get install -y docker.io docker-compose-plugin || apt-get install -y docker.io docker-compose

groupadd -f docker || true
DEFAULT_USER="$(getent passwd 1000 | cut -d: -f1 || true)"
if [ -n "${DEFAULT_USER}" ] && id "${DEFAULT_USER}" >/dev/null 2>&1; then
  usermod -aG docker "${DEFAULT_USER}" || true
fi

service docker start || true
if command -v systemctl >/dev/null 2>&1; then
  systemctl enable docker || true
  systemctl start docker || true
fi

docker version >/dev/null 2>&1
docker info >/dev/null 2>&1
docker compose version >/dev/null 2>&1 || docker-compose version >/dev/null 2>&1
"#;

    let output = std::process::Command::new("wsl")
        .args(["-d", distro, "-u", "root", "--", "bash", "-lc", script])
        .output()
        .map_err(|e| StageFailure {
            class: "engine_install_failed",
            message: format!(
                "Failed to run engine installation in WSL distro {distro}: {e}. Select Fix automatically to retry."
            ),
            retriable: true,
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let combined = format!("{stdout}\n{stderr}");

    if combined.contains("temporary failure resolving")
        || combined.contains("network is unreachable")
        || combined.contains("failed to fetch")
    {
        return Err(StageFailure {
            class: "engine_install_failed",
            message: "Network error during engine package installation. Check connectivity and select Fix automatically to retry.".to_string(),
            retriable: true,
        });
    }

    if combined.contains("permission denied")
        || combined.contains("access is denied")
        || combined.contains("administrator")
    {
        return Err(StageFailure {
            class: "permission_denied",
            message: "Engine installation requires elevated permissions. Re-run setup with admin rights, then select Fix automatically.".to_string(),
            retriable: false,
        });
    }

    Err(StageFailure {
        class: "engine_install_failed",
        message: "Engine package installation in WSL did not complete. Select Fix automatically to retry.".to_string(),
        retriable: true,
    })
}

#[cfg(target_os = "windows")]
fn verify_wsl_engine_socket_ready(distro: &str) -> Result<(), StageFailure> {
    let output = std::process::Command::new("wsl")
        .args([
            "-d",
            distro,
            "--",
            "bash",
            "-lc",
            "test -S /var/run/docker.sock && docker info >/dev/null 2>&1",
        ])
        .output()
        .map_err(|e| StageFailure {
            class: "relay_failed",
            message: format!(
                "Could not verify WSL Docker socket for relay setup: {e}. Select Fix automatically to retry."
            ),
            retriable: true,
        })?;

    if output.status.success() {
        return Ok(());
    }

    Err(StageFailure {
        class: "relay_failed",
        message:
            "WSL engine socket is not ready for relay registration. Select Fix automatically to retry."
                .to_string(),
        retriable: true,
    })
}

#[cfg(target_os = "windows")]
fn register_wsl_relay(app: &AppHandle, distro: &str, relay_pipe: &str) -> Result<(), StageFailure> {
    let data_dir = app.path().app_data_dir().map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not resolve app data path for relay registration: {e}"),
        retriable: false,
    })?;
    std::fs::create_dir_all(&data_dir).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not create relay registration directory: {e}"),
        retriable: false,
    })?;

    let relay_path = data_dir.join("wsl_relay_registration.json");
    let payload = serde_json::json!({
        "provider": "wsl_engine",
        "distro": distro,
        "relay_pipe": relay_pipe,
        "registered_at": Utc::now().to_rfc3339(),
    });
    let content = serde_json::to_string_pretty(&payload).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not serialize relay registration payload: {e}"),
        retriable: false,
    })?;
    std::fs::write(&relay_path, content).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not persist relay registration metadata: {e}"),
        retriable: false,
    })?;

    Ok(())
}

#[cfg(target_os = "windows")]
fn update_wsl_relay_state(
    app: &AppHandle,
    distro: &str,
    relay_pipe: &str,
    status: &str,
    last_error: Option<String>,
) -> Result<(), StageFailure> {
    let data_dir = app.path().app_data_dir().map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not resolve app data path for relay state: {e}"),
        retriable: false,
    })?;
    std::fs::create_dir_all(&data_dir).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not create relay state directory: {e}"),
        retriable: false,
    })?;

    let relay_state_path = data_dir.join("wsl_relay_state.json");
    let relay_state = WslRelayState {
        provider: "wsl_engine".to_string(),
        distro: distro.to_string(),
        relay_pipe: relay_pipe.to_string(),
        status: status.to_string(),
        last_checked_at: Utc::now().to_rfc3339(),
        last_error,
    };
    let content = serde_json::to_string_pretty(&relay_state).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not serialize relay state payload: {e}"),
        retriable: false,
    })?;
    std::fs::write(relay_state_path, content).map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not persist relay state metadata: {e}"),
        retriable: false,
    })?;
    Ok(())
}

#[cfg(target_os = "windows")]
async fn ensure_wsl_relay_registration_and_health(
    app: &AppHandle,
    distro: &str,
    relay_pipe: &str,
) -> Result<(), StageFailure> {
    register_wsl_relay(app, distro, relay_pipe)?;

    if can_ping(relay_pipe).await {
        update_wsl_relay_state(app, distro, relay_pipe, "running", None)?;
        return Ok(());
    }

    if let Some(fallback) = windows_fallback_endpoint() {
        if can_ping(&fallback).await {
            update_wsl_relay_state(
                app,
                distro,
                relay_pipe,
                "degraded",
                Some(format!(
                    "Managed relay unavailable, fallback endpoint in use: {fallback}"
                )),
            )?;
            return Ok(());
        }
    }

    let err = "Relay endpoint did not respond after registration.".to_string();
    update_wsl_relay_state(app, distro, relay_pipe, "degraded", Some(err))?;
    Err(StageFailure {
        class: "relay_failed",
        message: "Relay endpoint is registered but not reachable. Select Fix automatically to retry relay setup.".to_string(),
        retriable: true,
    })
}

#[cfg(target_os = "windows")]
fn windows_fallback_endpoint() -> Option<String> {
    let endpoint = resolve_socket_path().ok()?;
    if endpoint.trim().is_empty() {
        None
    } else {
        Some(endpoint)
    }
}

async fn validate_host_compatibility(endpoint: &str) -> Result<(), StageFailure> {
    let docker = connect_docker(endpoint).ok_or_else(|| StageFailure {
        class: "host_compat_failed",
        message: "Could not connect to Host Engine endpoint for compatibility checks."
            .to_string(),
        retriable: true,
    })?;

    docker.ping().await.map_err(|_| StageFailure {
        class: "host_compat_failed",
        message: "Host Engine ping failed during compatibility checks.".to_string(),
        retriable: true,
    })?;

    docker.version().await.map_err(|_| StageFailure {
        class: "host_compat_failed",
        message: "Host Engine version query failed during compatibility checks.".to_string(),
        retriable: true,
    })?;

    docker.info().await.map_err(|_| StageFailure {
        class: "host_compat_failed",
        message: "Host Engine info query failed during compatibility checks.".to_string(),
        retriable: true,
    })?;

    let probe_volume = format!("docker_gui_probe_{}", uuid::Uuid::new_v4());
    docker
        .create_volume(CreateVolumeOptions {
            name: probe_volume.clone(),
            driver: "local".to_string(),
            ..Default::default()
        })
        .await
        .map_err(|_| StageFailure {
            class: "host_compat_failed",
            message: "Host Engine write capability check failed (create volume).".to_string(),
            retriable: true,
        })?;

    docker
        .remove_volume(&probe_volume, Some(RemoveVolumeOptions { force: true }))
        .await
        .map_err(|_| StageFailure {
            class: "host_compat_failed",
            message: "Host Engine write capability check failed (remove volume).".to_string(),
            retriable: true,
        })?;

    if !host_compose_available() {
        return Err(StageFailure {
            class: "host_compat_failed",
            message: "Host Engine Compose compatibility check failed. Ensure `docker compose` is available."
                .to_string(),
            retriable: false,
        });
    }

    Ok(())
}

fn host_compose_available() -> bool {
    let v2 = std::process::Command::new("docker")
        .args(["compose", "version"])
        .output();
    if v2.map(|o| o.status.success()).unwrap_or(false) {
        return true;
    }

    let v1 = std::process::Command::new("docker-compose")
        .arg("version")
        .output();
    v1.map(|o| o.status.success()).unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn install_supported_wsl_distro() -> Result<(), StageFailure> {
    let output = std::process::Command::new("wsl")
        .args(["--install", "-d", "Ubuntu"])
        .output()
        .map_err(|e| StageFailure {
            class: "prereq_missing",
            message: format!(
                "Unable to start WSL distro installation: {e}. Select Fix automatically to retry."
            ),
            retriable: false,
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let combined = format!("{stdout}\n{stderr}").to_ascii_lowercase();

    if combined.contains("restart")
        || combined.contains("reboot")
        || combined.contains("0x80370102")
        || combined.contains("wsl must be updated")
    {
        return Err(StageFailure {
            class: "reboot_required",
            message: "Windows restart is required to complete WSL setup. Restart your device, then select Fix automatically to continue.".to_string(),
            retriable: false,
        });
    }

    if combined.contains("access is denied")
        || combined.contains("0x80070005")
        || combined.contains("administrator")
    {
        return Err(StageFailure {
            class: "permission_denied",
            message: "Administrator permissions are required to install WSL. Re-run setup with admin rights, then select Fix automatically.".to_string(),
            retriable: false,
        });
    }

    Err(StageFailure {
        class: "prereq_missing",
        message: "Could not install a supported WSL distro automatically. Ensure WSL is enabled, then select Fix automatically to retry.".to_string(),
        retriable: false,
    })
}

#[cfg(target_os = "windows")]
fn is_supported_ubuntu_distro(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "ubuntu" || normalized.starts_with("ubuntu-")
}
