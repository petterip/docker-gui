use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;

use crate::error::AppError;

pub const MANAGED_WSL_RELAY_PIPE: &str = "npipe:////./pipe/docker_gui_engine";
pub const DEFAULT_WSL_DISTRO: &str = "docker-gui-engine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HostEngineKind {
    #[serde(alias = "rancher_desktop_moby")]
    ExistingCompatibleHost,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum Provider {
    WslEngine { distro: String, relay_pipe: String },
    HostEngine { kind: HostEngineKind, endpoint: String },
    CustomHost { endpoint: String },
}

impl Provider {
    pub fn id(&self) -> &'static str {
        match self {
            Provider::WslEngine { .. } => "wsl_engine",
            Provider::HostEngine { .. } => "host_engine",
            Provider::CustomHost { .. } => "custom_host",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Provider::WslEngine { .. } => "WSL Engine",
            Provider::HostEngine { .. } => "Host Engine",
            Provider::CustomHost { .. } => "Custom Host",
        }
    }

    pub fn endpoint(&self) -> String {
        match self {
            Provider::WslEngine { relay_pipe, .. } => relay_pipe.clone(),
            Provider::HostEngine { endpoint, .. } => endpoint.clone(),
            Provider::CustomHost { endpoint } => endpoint.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct EngineConfig {
    pub active_provider: Option<Provider>,
    pub previous_provider: Option<Provider>,
    #[serde(default)]
    pub preferred_wsl_distro: Option<String>,
    pub resume_checkpoint: Option<String>,
    #[serde(default)]
    pub resume_privileged_allowed: bool,
    pub provisioning: Option<ProvisioningState>,
}

pub struct EngineRegistry(pub Mutex<EngineConfig>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningRunStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProvisioningStageStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningStage {
    pub id: String,
    pub label: String,
    pub status: ProvisioningStageStatus,
    pub failure_class: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisioningState {
    pub run_id: String,
    pub target_provider_id: String,
    pub status: ProvisioningRunStatus,
    pub stages: Vec<ProvisioningStage>,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

impl EngineRegistry {
    pub fn empty() -> Self {
        EngineRegistry(Mutex::new(EngineConfig::default()))
    }

    pub fn load(app: &AppHandle) -> Result<Self, AppError> {
        let path = engine_registry_path(app)?;
        let config = if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| AppError::RegistryError(e.to_string()))?;
            serde_json::from_str::<EngineConfig>(&content)
                .map_err(|e| AppError::RegistryError(e.to_string()))?
        } else {
            EngineConfig::default()
        };
        Ok(EngineRegistry(Mutex::new(config)))
    }

    pub async fn get(&self) -> EngineConfig {
        self.0.lock().await.clone()
    }

    pub async fn set_active_provider(
        &self,
        app: &AppHandle,
        provider: Provider,
    ) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        if guard.active_provider.as_ref() != Some(&provider) {
            guard.previous_provider = guard.active_provider.clone();
            guard.active_provider = Some(provider);
        }
        flush_atomic(app, &guard)
    }

    pub async fn set_resume_checkpoint(
        &self,
        app: &AppHandle,
        checkpoint: Option<String>,
    ) -> Result<(), AppError> {
        self.set_resume_checkpoint_with_privilege(app, checkpoint, false)
            .await
    }

    pub async fn set_resume_checkpoint_with_privilege(
        &self,
        app: &AppHandle,
        checkpoint: Option<String>,
        privileged_allowed: bool,
    ) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        guard.resume_checkpoint = checkpoint;
        guard.resume_privileged_allowed = guard.resume_checkpoint.is_some() && privileged_allowed;
        flush_atomic(app, &guard)
    }

    pub async fn set_preferred_wsl_distro(
        &self,
        app: &AppHandle,
        distro: Option<String>,
    ) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        guard.preferred_wsl_distro = distro;
        flush_atomic(app, &guard)
    }

    pub async fn clear_managed_wsl_engine(&self, app: &AppHandle) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;

        if matches!(guard.active_provider, Some(Provider::WslEngine { .. })) {
            guard.active_provider = guard.previous_provider.clone();
        }

        if matches!(guard.previous_provider, Some(Provider::WslEngine { .. })) {
            guard.previous_provider = None;
        }

        flush_atomic(app, &guard)
    }

    pub async fn begin_provisioning(
        &self,
        app: &AppHandle,
        run_id: String,
        target_provider_id: String,
        stages: Vec<ProvisioningStage>,
    ) -> Result<ProvisioningState, AppError> {
        let now = Utc::now().to_rfc3339();
        let mut guard = self.0.lock().await;
        let state = ProvisioningState {
            run_id,
            target_provider_id,
            status: ProvisioningRunStatus::Running,
            stages,
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
        };
        guard.provisioning = Some(state.clone());
        flush_atomic(app, &guard)?;
        Ok(state)
    }

    pub async fn mark_stage(
        &self,
        app: &AppHandle,
        stage_id: &str,
        status: ProvisioningStageStatus,
        failure_class: Option<String>,
        message: Option<String>,
    ) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        if let Some(p) = guard.provisioning.as_mut() {
            if let Some(stage) = p.stages.iter_mut().find(|s| s.id == stage_id) {
                stage.status = status;
                stage.failure_class = failure_class;
                stage.message = message;
                p.updated_at = Utc::now().to_rfc3339();
            }
        }
        flush_atomic(app, &guard)
    }

    pub async fn finish_provisioning(
        &self,
        app: &AppHandle,
        status: ProvisioningRunStatus,
    ) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        if let Some(p) = guard.provisioning.as_mut() {
            let now = Utc::now().to_rfc3339();
            p.status = status;
            p.updated_at = now.clone();
            p.finished_at = Some(now);
        }
        flush_atomic(app, &guard)
    }

}

fn engine_registry_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::create_dir_all(&data_dir).map_err(|e| AppError::RegistryError(e.to_string()))?;
    Ok(data_dir.join("engine_providers.json"))
}

fn flush_atomic(app: &AppHandle, config: &EngineConfig) -> Result<(), AppError> {
    let path = engine_registry_path(app)?;
    let tmp = path.with_extension("tmp");
    let content =
        serde_json::to_string_pretty(config).map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::write(&tmp, content).map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::rename(&tmp, &path).map_err(|e| AppError::RegistryError(e.to_string()))?;
    Ok(())
}
