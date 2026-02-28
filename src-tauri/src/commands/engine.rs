#![allow(dead_code, unused_variables)]

use chrono::Utc;
use bollard::volume::{CreateVolumeOptions, RemoveVolumeOptions};
use serde::{Deserialize, Serialize};
#[cfg(target_os = "windows")]
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::io::Read;
#[cfg(target_os = "windows")]
use std::path::{Path, PathBuf};
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

#[derive(Debug, Clone, Serialize)]
pub struct DiagnosticFile {
    pub path: String,
    pub exists: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineDiagnostics {
    pub bootstrapper_log: DiagnosticFile,
    pub helper_log: DiagnosticFile,
    pub reconnect_log: DiagnosticFile,
    pub engine_event_log: DiagnosticFile,
    pub relay_registration: DiagnosticFile,
    pub relay_state: DiagnosticFile,
}

#[derive(Debug, Serialize)]
pub struct EngineDiagnosticsExport {
    pub output_path: String,
    pub created_at: String,
    pub included_files: Vec<DiagnosticFile>,
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
    WslManagedDistroRemove,
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
            PrivilegedAction::WslManagedDistroRemove => "wsl_managed_distro_remove",
            PrivilegedAction::HostEngineDetect => "host_engine_detect",
            PrivilegedAction::HostCompatibilityValidate => "host_compatibility_validate",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
enum PrivilegedExecutionMode {
    HelperWithInProcessFallback,
    HelperRequiredNoFallback,
}

impl PrivilegedExecutionMode {
    fn as_str(self) -> &'static str {
        match self {
            PrivilegedExecutionMode::HelperWithInProcessFallback => {
                "helper_with_in_process_fallback"
            }
            PrivilegedExecutionMode::HelperRequiredNoFallback => "helper_required_no_fallback",
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

#[derive(Debug, Deserialize)]
pub struct SetWslDistroRequest {
    pub distro: String,
}

#[derive(Debug, Deserialize)]
pub struct RemoveManagedEngineRequest {
    pub remove_distro: bool,
    pub consent: bool,
}

#[derive(Debug, Serialize)]
pub struct WslDistroSelection {
    pub selected_distro: String,
    pub options: Vec<String>,
    pub recommended_distro: Option<String>,
}

#[tauri::command]
pub async fn get_engine_status(
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
) -> Result<EngineStatus, AppError> {
    let config = registry.get().await;
    let active_id = config.active_provider.as_ref().map(|p| p.id().to_string());

    let wsl_provider = provider_for_wsl(
        config.active_provider.as_ref(),
        config.preferred_wsl_distro.as_deref(),
    );
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
    source: Option<String>,
) -> Result<EngineStatus, AppError> {
    let target = match provider {
        InstallProviderRequest::WslEngine => {
            let config = registry.get().await;
            provider_for_wsl(None, config.preferred_wsl_distro.as_deref())
        }
        InstallProviderRequest::HostEngine => {
            return Err(AppError::InvalidArgument(
                "Host Engine installation is disabled by policy. Use Switch now to connect an existing compatible host, or install WSL Engine."
                    .to_string(),
            ))
        }
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "install_engine_provider")?;
    start_provisioning_run(app, state, registry, target, None, source).await
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
        SwitchProviderRequest::WslEngine => provider_for_wsl(
            config.active_provider.as_ref(),
            config.preferred_wsl_distro.as_deref(),
        ),
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
        #[cfg(target_os = "windows")]
        match &provider {
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
        }

        #[cfg(not(target_os = "windows"))]
        match &provider {
            Provider::WslEngine { .. } => {}
            Provider::HostEngine { endpoint, .. } | Provider::CustomHost { endpoint } => {
                validate_host_compatibility(endpoint)
                    .await
                    .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
            }
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
pub async fn list_wsl_engine_distros(
    registry: State<'_, EngineRegistry>,
) -> Result<WslDistroSelection, AppError> {
    let config = registry.get().await;
    let selected_distro = match config.active_provider.as_ref() {
        Some(Provider::WslEngine { distro, .. }) => distro.clone(),
        _ => config
            .preferred_wsl_distro
            .clone()
            .or_else(discover_preferred_wsl_distro)
            .unwrap_or_else(|| DEFAULT_WSL_DISTRO.to_string()),
    };

    #[cfg(target_os = "windows")]
    let installed = list_wsl_distros()
        .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
    #[cfg(not(target_os = "windows"))]
    let installed: Vec<String> = Vec::new();

    let mut options = Vec::<String>::new();
    options.push(DEFAULT_WSL_DISTRO.to_string());
    for distro in installed {
        if distro.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO) || is_supported_ubuntu_distro(&distro) {
            options.push(distro);
        }
    }
    options.sort_unstable();
    options.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    if !options.iter().any(|d| d.eq_ignore_ascii_case(&selected_distro)) {
        options.push(selected_distro.clone());
    }

    let recommended_distro = options
        .iter()
        .find(|d| d.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO))
        .cloned()
        .or_else(|| options.first().cloned());

    Ok(WslDistroSelection {
        selected_distro,
        options,
        recommended_distro,
    })
}

#[tauri::command]
pub async fn set_wsl_engine_distro(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    request: SetWslDistroRequest,
) -> Result<EngineStatus, AppError> {
    let selected = request.distro.trim();
    if selected.is_empty() {
        return Err(AppError::InvalidArgument(
            "WSL distro selection cannot be empty.".to_string(),
        ));
    }

    #[cfg(target_os = "windows")]
    {
        if !selected.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO) {
            let installed = list_wsl_distros()
                .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
            let installed_match = installed
                .iter()
                .any(|name| name.eq_ignore_ascii_case(selected));
            if !installed_match {
                return Err(AppError::InvalidArgument(
                    "Selected WSL distro is not installed. Install it first, then retry."
                        .to_string(),
                ));
            }
            if !is_supported_ubuntu_distro(selected) {
                return Err(AppError::InvalidArgument(
                    "Only supported Ubuntu distros can be selected for WSL Engine."
                        .to_string(),
                ));
            }
        }
    }

    registry
        .set_preferred_wsl_distro(&app, Some(selected.to_string()))
        .await?;
    let config = registry.get().await;
    if let Some(Provider::WslEngine { relay_pipe, .. }) = config.active_provider {
        registry
            .set_active_provider(
                &app,
                Provider::WslEngine {
                    distro: selected.to_string(),
                    relay_pipe,
                },
            )
            .await?;
        state
            .set_preferred_endpoint(Some(MANAGED_WSL_RELAY_PIPE.to_string()))
            .await;
    }
    emit_engine_event(
        &app,
        "wsl_distro_selected",
        serde_json::json!({ "distro": selected }),
    );

    get_engine_status(state, registry).await
}

#[tauri::command]
pub async fn get_engine_diagnostics(app: AppHandle) -> Result<EngineDiagnostics, AppError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    let logs_dir = data_dir.join("logs");

    Ok(EngineDiagnostics {
        bootstrapper_log: diagnostic_file(logs_dir.join("bootstrapper.log")),
        helper_log: diagnostic_file(logs_dir.join("provisioning-helper.log")),
        reconnect_log: diagnostic_file(logs_dir.join("reconnect.log")),
        engine_event_log: diagnostic_file(logs_dir.join("engine-events.jsonl")),
        relay_registration: diagnostic_file(data_dir.join("wsl_relay_registration.json")),
        relay_state: diagnostic_file(data_dir.join("wsl_relay_state.json")),
    })
}

#[tauri::command]
pub async fn export_engine_diagnostics(
    app: AppHandle,
    registry: State<'_, EngineRegistry>,
) -> Result<EngineDiagnosticsExport, AppError> {
    let diagnostics = get_engine_diagnostics(app.clone()).await?;
    let config = registry.get().await;
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    let exports_dir = data_dir.join("diagnostics");
    std::fs::create_dir_all(&exports_dir).map_err(|e| AppError::RegistryError(e.to_string()))?;

    let created_at = Utc::now();
    let stamp = created_at.format("%Y%m%dT%H%M%SZ").to_string();
    let output_path = exports_dir.join(format!("engine-diagnostics-{stamp}.json"));
    let events_tail = read_tail_lines(&diagnostics.engine_event_log.path, 200)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;

    let included_files = vec![
        diagnostics.bootstrapper_log.clone(),
        diagnostics.helper_log.clone(),
        diagnostics.reconnect_log.clone(),
        diagnostics.engine_event_log.clone(),
        diagnostics.relay_registration.clone(),
        diagnostics.relay_state.clone(),
    ];

    let payload = serde_json::json!({
        "created_at": created_at.to_rfc3339(),
        "engine_registry": config,
        "diagnostics": diagnostics,
        "engine_event_tail": events_tail,
    });

    let content = serde_json::to_string_pretty(&payload)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::write(&output_path, content).map_err(|e| AppError::RegistryError(e.to_string()))?;

    emit_engine_event(
        &app,
        "diagnostics_exported",
        serde_json::json!({
            "output_path": output_path.to_string_lossy(),
            "included_count": included_files.iter().filter(|f| f.exists).count(),
        }),
    );

    Ok(EngineDiagnosticsExport {
        output_path: output_path.to_string_lossy().to_string(),
        created_at: created_at.to_rfc3339(),
        included_files,
    })
}

#[tauri::command]
pub async fn get_privileged_action_contract() -> Result<PrivilegedActionContract, AppError> {
    let execution_mode = if helper_strict_mode_enabled() {
        PrivilegedExecutionMode::HelperRequiredNoFallback
    } else {
        PrivilegedExecutionMode::HelperWithInProcessFallback
    };
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
            id: PrivilegedAction::WslManagedDistroRemove.id().to_string(),
            description: "Remove the managed WSL engine distro lifecycle resources.".to_string(),
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
    request: RemoveManagedEngineRequest,
) -> Result<EngineStatus, AppError> {
    if request.remove_distro {
        let managed_provider = Provider::WslEngine {
            distro: DEFAULT_WSL_DISTRO.to_string(),
            relay_pipe: MANAGED_WSL_RELAY_PIPE.to_string(),
        };
        ensure_privileged_consent_for_provider(
            &app,
            &managed_provider,
            request.consent,
            "remove_managed_engine",
        )?;
        run_privileged_action(
            &app,
            PrivilegedAction::WslManagedDistroRemove,
            &managed_provider,
            "remove_managed_engine",
        )
        .await
        .map_err(|failure| AppError::InvalidArgument(failure.message.clone()))?;
    }

    registry.clear_managed_wsl_engine(&app).await?;
    let config = registry.get().await;
    state
        .set_preferred_endpoint(config.active_provider.map(|p| p.endpoint()))
        .await;
    emit_engine_event(
        &app,
        "managed_engine_removed",
        serde_json::json!({ "remove_distro": request.remove_distro }),
    );
    get_engine_status(state, registry).await
}

#[tauri::command]
pub async fn start_engine_provisioning(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    provider: InstallProviderRequest,
    consent: bool,
    source: Option<String>,
) -> Result<ProvisioningState, AppError> {
    let target = match provider {
        InstallProviderRequest::WslEngine => {
            let config = registry.get().await;
            provider_for_wsl(None, config.preferred_wsl_distro.as_deref())
        }
        InstallProviderRequest::HostEngine => {
            return Err(AppError::InvalidArgument(
                "Host Engine installation is disabled by policy. Use Switch now to connect an existing compatible host, or install WSL Engine."
                    .to_string(),
            ))
        }
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "start_engine_provisioning")?;
    let status = start_provisioning_run(app, state, registry, target, None, source).await?;
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
    source: Option<String>,
) -> Result<ProvisioningState, AppError> {
    let config = registry.get().await;
    let provisioning = config
        .provisioning
        .ok_or_else(|| AppError::InvalidArgument("No provisioning run available to retry.".into()))?;
    let resume_checkpoint = config.resume_checkpoint.clone();
    let target = match provisioning.target_provider_id.as_str() {
        "wsl_engine" => provider_for_wsl(
            config.active_provider.as_ref(),
            config.preferred_wsl_distro.as_deref(),
        ),
        "host_engine" => provider_for_host(config.active_provider.as_ref()),
        _ => {
            return Err(AppError::InvalidArgument(
                "Retry is only supported for WSL Engine or Host Engine.".into(),
            ))
        }
    };
    ensure_privileged_consent_for_provider(&app, &target, consent, "retry_engine_provisioning")?;

    emit_engine_event(
        &app,
        "provisioning_retry_requested",
        serde_json::json!({
            "resume_checkpoint": resume_checkpoint,
            "target_provider": target.id(),
            "source": sanitize_provisioning_source(source.as_deref()),
        }),
    );
    let status = start_provisioning_run(
        app,
        state,
        registry,
        target,
        resume_checkpoint,
        source,
    )
    .await?;
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
        Some("wsl_engine") => provider_for_wsl(
            config.active_provider.as_ref(),
            config.preferred_wsl_distro.as_deref(),
        ),
        Some("host_engine") => provider_for_host(config.active_provider.as_ref()),
        Some(other) => {
            return Err(AppError::InvalidArgument(format!(
                "Unknown provisioning target for resume: {other}"
            )))
        }
        None => match config.active_provider.as_ref().map(|p| p.id()) {
            Some("wsl_engine") => provider_for_wsl(
                config.active_provider.as_ref(),
                config.preferred_wsl_distro.as_deref(),
            ),
            Some("host_engine") => provider_for_host(config.active_provider.as_ref()),
            _ => return Ok(None),
        },
    };

    if provider_requires_elevation(&target) && !config.resume_privileged_allowed {
        emit_engine_event(
            &app,
            "provisioning_resume_blocked_consent",
            serde_json::json!({
                "resume_checkpoint": resume_checkpoint,
                "target_provider": target.id(),
            }),
        );
        return Ok(config.provisioning);
    }

    emit_engine_event(
        &app,
        "provisioning_resume_requested",
        serde_json::json!({
            "resume_checkpoint": resume_checkpoint,
            "target_provider": target.id(),
        }),
    );
    let status =
        start_provisioning_run(
            app.clone(),
            state,
            registry,
            target,
            Some(resume_checkpoint),
            Some("resume_engine_provisioning_if_needed".to_string()),
        )
        .await?;
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

fn provider_for_wsl(active: Option<&Provider>, preferred: Option<&str>) -> Provider {
    if let Some(Provider::WslEngine { distro, relay_pipe }) = active {
        return Provider::WslEngine {
            distro: distro.clone(),
            relay_pipe: relay_pipe.clone(),
        };
    }

    let preferred = preferred
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(ToString::to_string);
    let discovered_distro = preferred
        .or_else(discover_preferred_wsl_distro)
        .unwrap_or_else(|| DEFAULT_WSL_DISTRO.to_string());

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

fn diagnostic_file(path: std::path::PathBuf) -> DiagnosticFile {
    DiagnosticFile {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
    }
}

fn read_tail_lines(path: &str, max_lines: usize) -> Result<Vec<String>, std::io::Error> {
    if max_lines == 0 {
        return Ok(Vec::new());
    }

    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };

    let mut lines = content
        .lines()
        .map(ToString::to_string)
        .collect::<Vec<String>>();
    if lines.len() > max_lines {
        let start = lines.len() - max_lines;
        lines = lines.split_off(start);
    }
    Ok(lines)
}

async fn start_provisioning_run(
    app: AppHandle,
    state: State<'_, AppState>,
    registry: State<'_, EngineRegistry>,
    target: Provider,
    start_stage: Option<String>,
    source: Option<String>,
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
    let source_tag = sanitize_provisioning_source(source.as_deref()).to_string();
    let resume_privileged_allowed = provider_requires_elevation(&target);
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
            "source": source_tag.clone(),
        }),
    );
    if let Some(first) = stage_ids.get(start_index) {
        registry
            .set_resume_checkpoint_with_privilege(
                &app,
                Some((*first).to_string()),
                resume_privileged_allowed,
            )
            .await?;
    }

    let endpoint = target.endpoint();
    state.set_preferred_endpoint(Some(endpoint)).await;

    let app_for_task = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = run_provisioning(app_for_task, target, start_index, source_tag).await;
    });

    get_engine_status(state, registry).await
}

async fn run_provisioning(
    app: AppHandle,
    target: Provider,
    start_index: usize,
    source: String,
) -> Result<(), AppError> {
    let stage_ids = provisioning_stage_ids(&target);
    let resume_privileged_allowed = provider_requires_elevation(&target);
    for (idx, stage_id) in stage_ids.iter().enumerate().skip(start_index) {
        emit_engine_event(
            &app,
            "provisioning_stage_started",
            serde_json::json!({ "stage_id": stage_id, "target_provider_id": target.id() }),
        );
        update_stage(&app, stage_id, ProvisioningStageStatus::InProgress, None, None).await?;
        if let Err(failure) = execute_stage_with_backoff(&app, &target, stage_id).await {
            let failure_message = failure.message.clone();
            let canonical_failure_class = canonical_failure_class(failure.class);
            update_stage(
                &app,
                stage_id,
                ProvisioningStageStatus::Failed,
                Some(canonical_failure_class.to_string()),
                Some(failure_message.clone()),
            )
            .await?;
            let registry = app.state::<EngineRegistry>();
            registry.finish_provisioning(&app, ProvisioningRunStatus::Failed).await?;
            registry
                .set_resume_checkpoint_with_privilege(
                    &app,
                    Some(stage_id.to_string()),
                    resume_privileged_allowed,
                )
                .await?;
            emit_engine_event(
                &app,
                "provisioning_stage_failed",
                serde_json::json!({
                    "stage_id": stage_id,
                    "target_provider_id": target.id(),
                    "failure_class": canonical_failure_class,
                    "raw_failure_class": failure.class,
                    "message": failure_message,
                }),
            );
            emit_engine_event(
                &app,
                "provisioning_failed",
                serde_json::json!({
                    "target_provider_id": target.id(),
                    "failed_stage": stage_id,
                    "failure_class": canonical_failure_class,
                    "raw_failure_class": failure.class,
                    "source": &source,
                }),
            );
            emit_engine_event(
                &app,
                "provider_install_failed",
                serde_json::json!({
                    "provider_id": target.id(),
                    "failed_stage": stage_id,
                    "failure_class": canonical_failure_class,
                    "raw_failure_class": failure.class,
                    "source": &source,
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
        registry
            .set_resume_checkpoint_with_privilege(
                &app,
                next_checkpoint,
                resume_privileged_allowed,
            )
            .await?;
    }

    let registry = app.state::<EngineRegistry>();
    let target_provider_id = target.id().to_string();
    registry.set_active_provider(&app, target).await?;
    registry.finish_provisioning(&app, ProvisioningRunStatus::Succeeded).await?;
    registry.set_resume_checkpoint(&app, None).await?;
    emit_engine_event(
        &app,
        "provisioning_succeeded",
        serde_json::json!({ "target_provider_id": target_provider_id.clone(), "source": &source }),
    );
    emit_engine_event(
        &app,
        "provider_installed",
        serde_json::json!({
            "provider_id": target_provider_id,
            "source": &source,
        }),
    );
    Ok(())
}

fn sanitize_provisioning_source(source: Option<&str>) -> &'static str {
    match source.map(str::trim).filter(|s| !s.is_empty()) {
        Some("settings_engine_install") => "settings_engine_install",
        Some("settings_engine_retry") => "settings_engine_retry",
        Some("install_engine_provider") => "install_engine_provider",
        Some("start_engine_provisioning") => "start_engine_provisioning",
        Some("retry_engine_provisioning") => "retry_engine_provisioning",
        Some("resume_engine_provisioning_if_needed") => "resume_engine_provisioning_if_needed",
        _ => "unspecified",
    }
}

fn canonical_failure_class(raw: &str) -> &'static str {
    match raw {
        "prereq_missing" => "prereq_missing",
        "reboot_required" => "reboot_required",
        "distro_install_failed" => "distro_install_failed",
        "engine_install_failed" => "engine_install_failed",
        "engine_start_failed" => "engine_start_failed",
        "relay_failed" => "relay_failed",
        "connectivity_failed" => "connectivity_failed",
        "permission_denied" => "connectivity_failed",
        "host_not_installed" => "prereq_missing",
        "host_compat_failed" => "connectivity_failed",
        "host_policy_blocked" => "prereq_missing",
        "helper_failed" => "connectivity_failed",
        "distro_remove_failed" => "distro_install_failed",
        _ => "connectivity_failed",
    }
}

fn helper_strict_mode_enabled() -> bool {
    if let Some(strict) = parse_bool_env("DOCKER_GUI_HELPER_STRICT") {
        return strict;
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(allow_fallback) = parse_bool_env("DOCKER_GUI_ALLOW_IN_PROCESS_FALLBACK") {
            return !allow_fallback;
        }
        // Windows provisioning is expected to run helper-first in production.
        return true;
    }

    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

fn parse_bool_env(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|v| {
        let normalized = v.trim().to_ascii_lowercase();
        matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
    })
}

#[cfg(target_os = "windows")]
fn resolve_helper_binary_path(app: Option<&AppHandle>) -> Option<PathBuf> {
    const CANDIDATE_NAMES: [&str; 2] = [
        "docker-gui-provisioning-helper.exe",
        "docker-gui-provisioning-helper",
    ];

    if let Ok(explicit) = std::env::var("DOCKER_GUI_HELPER_PATH") {
        let candidate = PathBuf::from(explicit.trim());
        if candidate.exists() {
            return Some(candidate);
        }
    }

    if let Some(app_handle) = app {
        if let Ok(resource_dir) = app_handle.path().resource_dir() {
            for sub in ["bin/win32", "bin"] {
                let base = if sub.is_empty() {
                    resource_dir.clone()
                } else {
                    resource_dir.join(sub)
                };
                for file in CANDIDATE_NAMES {
                    let candidate = base.join(file);
                    if candidate.exists() {
                        return Some(candidate);
                    }
                }
            }
        }
    }

    if let Some(manifest_dir) = option_env!("CARGO_MANIFEST_DIR") {
        let base = PathBuf::from(manifest_dir).join("bin");
        let win32 = base.join("win32");
        for search_dir in [win32, base] {
            for file in CANDIDATE_NAMES {
                let candidate = search_dir.join(file);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(parent) = current_exe.parent() {
            for file in CANDIDATE_NAMES {
                let candidate = parent.join(file);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    if let Ok(path_env) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_env) {
            for file in CANDIDATE_NAMES {
                let candidate = dir.join(file);
                if candidate.exists() {
                    return Some(candidate);
                }
            }
        }
    }

    None
}

#[cfg(target_os = "windows")]
fn helper_checksum_required() -> bool {
    parse_bool_env("DOCKER_GUI_HELPER_ENFORCE_CHECKSUM").unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn verify_helper_binary_integrity(helper_path: &Path) -> Result<(), String> {
    let expected = expected_helper_sha256(helper_path);
    let expected = match expected {
        Some(value) => value,
        None => {
            if helper_checksum_required() {
                return Err("helper_checksum_missing".to_string());
            }
            return Ok(());
        }
    };

    let mut file =
        std::fs::File::open(helper_path).map_err(|e| format!("helper_read_failed:{e}"))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 16 * 1024];
    loop {
        let read = file
            .read(&mut buf)
            .map_err(|e| format!("helper_read_failed:{e}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
    }
    let actual = format!("{:x}", hasher.finalize());

    if actual.eq_ignore_ascii_case(&expected) {
        Ok(())
    } else {
        Err(format!("helper_checksum_mismatch expected={expected} actual={actual}"))
    }
}

#[cfg(target_os = "windows")]
fn expected_helper_sha256(helper_path: &Path) -> Option<String> {
    if let Ok(from_env) = std::env::var("DOCKER_GUI_HELPER_SHA256") {
        let normalized = normalize_sha256_hex(from_env.trim())?;
        return Some(normalized);
    }

    let sidecar_candidates = [
        helper_path.with_extension("sha256"),
        helper_path.with_file_name("docker-gui-provisioning-helper.sha256"),
    ];
    for candidate in sidecar_candidates {
        let content = match std::fs::read_to_string(candidate) {
            Ok(content) => content,
            Err(_) => continue,
        };
        for token in content.split_whitespace() {
            if let Some(normalized) = normalize_sha256_hex(token) {
                return Some(normalized);
            }
        }
    }
    None
}

fn normalize_sha256_hex(value: &str) -> Option<String> {
    let normalized = value.trim().to_ascii_lowercase();
    if normalized.len() != 64 || !normalized.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(normalized)
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

    let execution_result = match try_execute_helper_action(app, action, target) {
        Ok(HelperDispatch::Handled(result)) => Ok(result),
        Ok(HelperDispatch::Fallback { reason }) => {
            if helper_strict_mode_enabled() {
                execution_mode = "helper_required_no_fallback";
                Err(StageFailure {
                    class: "helper_failed",
                    message: format!(
                        "Privileged helper is required but unavailable: {reason}. Install/enable docker-gui-provisioning-helper and retry."
                    ),
                    retriable: true,
                })
            } else {
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
    app: &AppHandle,
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
        let helper_path = match resolve_helper_binary_path(Some(app)) {
            Some(path) => path,
            None => {
                return Ok(HelperDispatch::Fallback {
                    reason: "helper_binary_not_found".to_string(),
                });
            }
        };
        if let Err(reason) = verify_helper_binary_integrity(&helper_path) {
            return Ok(HelperDispatch::Fallback {
                reason: format!("helper_integrity_check_failed:{reason}"),
            });
        }

        let target_json = serde_json::to_string(target).map_err(|e| StageFailure {
            class: "helper_failed",
            message: format!("Could not serialize helper action payload: {e}"),
            retriable: false,
        })?;
        let app_data_dir = app_data_dir_for_helper(app)?;

        let output = match std::process::Command::new(&helper_path)
            .args([
                "run-action",
                "--action",
                action.id(),
                "--target-provider",
                target.id(),
                "--target-json",
                &target_json,
                "--app-data-dir",
                &app_data_dir,
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
        "distro_remove_failed" => "distro_remove_failed",
        _ => "helper_failed",
    }
}

fn app_data_dir_for_helper(app: &AppHandle) -> Result<String, StageFailure> {
    let path = app.path().app_data_dir().map_err(|e| StageFailure {
        class: "helper_failed",
        message: format!("Could not resolve app data directory for helper: {e}"),
        retriable: false,
    })?;
    Ok(path.to_string_lossy().to_string())
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
        PrivilegedAction::WslManagedDistroRemove => {
            unregister_managed_wsl_distro()?;
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
        Provider::HostEngine { endpoint, .. } => {
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
    ensure_wsl_engine_runtime_running(distro)?;

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
fn ensure_wsl_engine_runtime_running(distro: &str) -> Result<(), StageFailure> {
    let output = std::process::Command::new("wsl")
        .args([
            "-d",
            distro,
            "-u",
            "root",
            "--",
            "bash",
            "-lc",
            r#"set -e
service docker start >/dev/null 2>&1 || true
if command -v systemctl >/dev/null 2>&1; then
  systemctl start docker >/dev/null 2>&1 || true
fi
docker info >/dev/null 2>&1"#,
        ])
        .output()
        .map_err(|e| StageFailure {
            class: "engine_start_failed",
            message: format!(
                "Could not start Docker engine service in WSL distro {distro}: {e}. Select Fix automatically to retry."
            ),
            retriable: true,
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let combined = format!("{stdout}\n{stderr}");

    if combined.contains("permission denied")
        || combined.contains("access is denied")
        || combined.contains("administrator")
    {
        return Err(StageFailure {
            class: "permission_denied",
            message: "Administrator permissions are required to start Docker service in WSL. Re-run setup with admin rights, then select Fix automatically."
                .to_string(),
            retriable: false,
        });
    }

    Err(StageFailure {
        class: "engine_start_failed",
        message:
            "Docker engine service in WSL is not running. Select Fix automatically to retry."
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

    let started_relay = start_managed_wsl_relay_process(app, distro, relay_pipe)?;
    if started_relay {
        if wait_for_relay_pipe(relay_pipe, 12, 500).await {
            update_wsl_relay_state(app, distro, relay_pipe, "running", None)?;
            return Ok(());
        }
        update_wsl_relay_state(
            app,
            distro,
            relay_pipe,
            "degraded",
            Some("Managed relay start command did not produce a reachable endpoint.".to_string()),
        )?;
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

#[cfg(target_os = "windows")]
fn start_managed_wsl_relay_process(
    app: &AppHandle,
    distro: &str,
    relay_pipe: &str,
) -> Result<bool, StageFailure> {
    let app_data_dir = app.path().app_data_dir().map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not resolve app data directory for relay startup: {e}"),
        retriable: false,
    })?;
    let app_data_dir = app_data_dir.to_string_lossy().to_string();

    if let Some(template) = relay_start_command_template() {
        let command = render_relay_start_command(&template, distro, relay_pipe, &app_data_dir);
        let result = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &command,
            ])
            .spawn();
        match result {
            Ok(_) => return Ok(true),
            Err(e) => {
                return Err(StageFailure {
                    class: "relay_failed",
                    message: format!(
                        "Could not start managed relay process with configured command: {e}"
                    ),
                    retriable: true,
                });
            }
        }
    }

    if let Some(helper_binary) = resolve_helper_binary_path(Some(app)) {
        if let Err(reason) = verify_helper_binary_integrity(&helper_binary) {
            return Err(StageFailure {
                class: "relay_failed",
                message: format!("Built-in helper relay integrity check failed: {reason}"),
                retriable: false,
            });
        }
        let result = std::process::Command::new(helper_binary)
            .args([
                "run-relay",
                "--distro",
                distro,
                "--pipe",
                relay_pipe,
                "--app-data-dir",
                &app_data_dir,
            ])
            .spawn();
        match result {
            Ok(_) => return Ok(true),
            Err(e) => {
                return Err(StageFailure {
                    class: "relay_failed",
                    message: format!("Could not start built-in helper relay process: {e}"),
                    retriable: true,
                });
            }
        }
    }

    let current_exe = std::env::current_exe().map_err(|e| StageFailure {
        class: "relay_failed",
        message: format!("Could not resolve current executable for relay startup: {e}"),
        retriable: true,
    })?;
    if let Some(parent) = current_exe.parent() {
        for candidate in [
            "docker-gui-wsl-relay.exe",
            "docker-gui-wsl-relay",
            "docker-gui-relay.exe",
            "docker-gui-relay",
        ] {
            let relay_exe = parent.join(candidate);
            if !relay_exe.exists() {
                continue;
            }
            let result = std::process::Command::new(relay_exe)
                .args([
                    "--distro",
                    distro,
                    "--pipe",
                    relay_pipe,
                    "--app-data-dir",
                    &app_data_dir,
                ])
                .spawn();
            match result {
                Ok(_) => return Ok(true),
                Err(e) => {
                    return Err(StageFailure {
                        class: "relay_failed",
                        message: format!("Could not launch managed relay executable: {e}"),
                        retriable: true,
                    });
                }
            }
        }
    }

    Ok(false)
}

#[cfg(target_os = "windows")]
fn relay_start_command_template() -> Option<String> {
    std::env::var("DOCKER_GUI_WSL_RELAY_START_CMD")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(target_os = "windows")]
fn render_relay_start_command(
    template: &str,
    distro: &str,
    relay_pipe: &str,
    app_data_dir: &str,
) -> String {
    template
        .replace("{distro}", distro)
        .replace("{relay_pipe}", relay_pipe)
        .replace("{app_data_dir}", app_data_dir)
}

#[cfg(target_os = "windows")]
async fn wait_for_relay_pipe(relay_pipe: &str, attempts: usize, delay_ms: u64) -> bool {
    for _ in 0..attempts {
        if can_ping(relay_pipe).await {
            return true;
        }
        sleep(Duration::from_millis(delay_ms)).await;
    }
    false
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
fn unregister_managed_wsl_distro() -> Result<(), StageFailure> {
    let distros = list_wsl_distros()?;
    if !distros
        .iter()
        .any(|name| name.eq_ignore_ascii_case(DEFAULT_WSL_DISTRO))
    {
        return Ok(());
    }

    let output = std::process::Command::new("wsl")
        .args(["--unregister", DEFAULT_WSL_DISTRO])
        .output()
        .map_err(|e| StageFailure {
            class: "distro_remove_failed",
            message: format!(
                "Could not remove the managed WSL engine distro: {e}. Select Remove managed engine again to retry."
            ),
            retriable: true,
        })?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).to_ascii_lowercase();
    let stdout = String::from_utf8_lossy(&output.stdout).to_ascii_lowercase();
    let combined = format!("{stdout}\n{stderr}");

    if combined.contains("not found")
        || combined.contains("there is no distribution")
        || combined.contains("was not found")
    {
        return Ok(());
    }

    if combined.contains("access is denied")
        || combined.contains("0x80070005")
        || combined.contains("administrator")
        || combined.contains("elevation")
    {
        return Err(StageFailure {
            class: "permission_denied",
            message: "Administrator permissions are required to remove the managed engine distro. Approve the permission prompt and retry."
                .to_string(),
            retriable: false,
        });
    }

    Err(StageFailure {
        class: "distro_remove_failed",
        message: "Managed engine removal did not complete. Select Remove managed engine again to retry."
            .to_string(),
        retriable: true,
    })
}

#[cfg(not(target_os = "windows"))]
fn unregister_managed_wsl_distro() -> Result<(), StageFailure> {
    Ok(())
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

fn is_supported_ubuntu_distro(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    normalized == "ubuntu" || normalized.starts_with("ubuntu-")
}
