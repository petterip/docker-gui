use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::error::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stack {
    pub id: String,
    pub name: String,
    pub compose_file: String,
    pub added_at: DateTime<Utc>,
    #[serde(default)]
    pub missing: bool,
}

pub struct StacksRegistry(pub Mutex<Vec<Stack>>);

impl StacksRegistry {
    pub fn empty() -> Self {
        StacksRegistry(Mutex::new(Vec::new()))
    }

    pub fn load(app: &AppHandle) -> Result<Self, AppError> {
        let path = registry_path(app)?;
        let stacks = if path.exists() {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| AppError::RegistryError(e.to_string()))?;
            let mut stacks: Vec<Stack> = serde_json::from_str(&content)
                .map_err(|e| AppError::RegistryError(e.to_string()))?;
            // Mark stacks whose compose file no longer exists
            for s in &mut stacks {
                s.missing = !std::path::Path::new(&s.compose_file).exists();
            }
            stacks
        } else {
            Vec::new()
        };
        Ok(StacksRegistry(Mutex::new(stacks)))
    }

    pub async fn add(&self, app: &AppHandle, name: String, compose_file: String) -> Result<Stack, AppError> {
        let stack = Stack {
            id: Uuid::new_v4().to_string(),
            name,
            compose_file,
            added_at: Utc::now(),
            missing: false,
        };
        let mut guard = self.0.lock().await;
        guard.push(stack.clone());
        flush_atomic(app, &guard)?;
        Ok(stack)
    }

    pub async fn remove(&self, app: &AppHandle, id: &str) -> Result<(), AppError> {
        let mut guard = self.0.lock().await;
        let before = guard.len();
        guard.retain(|s| s.id != id);
        if guard.len() == before {
            return Err(AppError::StackNotFound(id.to_string()));
        }
        flush_atomic(app, &guard)
    }

    pub async fn get_all(&self) -> Vec<Stack> {
        self.0.lock().await.clone()
    }

    pub async fn get_by_id(&self, id: &str) -> Option<Stack> {
        self.0.lock().await.iter().find(|s| s.id == id).cloned()
    }
}

fn registry_path(app: &AppHandle) -> Result<PathBuf, AppError> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::create_dir_all(&data_dir)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    Ok(data_dir.join("stacks.json"))
}

fn flush_atomic(app: &AppHandle, stacks: &[Stack]) -> Result<(), AppError> {
    let path = registry_path(app)?;
    let tmp = path.with_extension("tmp");
    let content = serde_json::to_string_pretty(stacks)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::write(&tmp, content)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    std::fs::rename(&tmp, &path)
        .map_err(|e| AppError::RegistryError(e.to_string()))?;
    Ok(())
}
