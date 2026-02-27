use bollard::image::{CreateImageOptions, ListImagesOptions, RemoveImageOptions};
use bollard::models::ImageSummary;
use futures_util::StreamExt;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::config::AppState;
use crate::error::AppError;

#[derive(Debug, Serialize, Clone)]
pub struct ImageItem {
    pub id: String,
    pub repo_tags: Vec<String>,
    pub size: i64,
    pub created: i64,
    pub dangling: bool,
}

fn map_image(img: ImageSummary) -> ImageItem {
    let dangling = img.repo_tags.iter().any(|t| t == "<none>:<none>") || img.repo_tags.is_empty();
    ImageItem {
        id: img.id,
        repo_tags: img.repo_tags,
        size: img.size,
        created: img.created,
        dangling,
    }
}

#[tauri::command]
pub async fn list_images(state: State<'_, AppState>) -> Result<Vec<ImageItem>, AppError> {
    let docker = state.get_docker().await?;
    let options = ListImagesOptions::<String> {
        all: false,
        ..Default::default()
    };
    let images = docker.list_images(Some(options)).await?;
    Ok(images.into_iter().map(map_image).collect())
}

#[tauri::command]
pub async fn remove_image(
    id: String,
    force: bool,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    docker
        .remove_image(
            &id,
            Some(RemoveImageOptions { force, noprune: false }),
            None,
        )
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn pull_image(
    name: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let docker = state.get_docker().await?;
    let options = CreateImageOptions {
        from_image: name.clone(),
        ..Default::default()
    };
    let mut stream = docker.create_image(Some(options), None, None);

    while let Some(result) = stream.next().await {
        match result {
            Ok(info) => {
                let _ = app.emit("image-pull-progress", &info);
            }
            Err(e) => return Err(AppError::from(e)),
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn inspect_image(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, AppError> {
    let docker = state.get_docker().await?;
    let info = docker.inspect_image(&id).await?;
    Ok(serde_json::to_value(info).map_err(AppError::from)?)
}
