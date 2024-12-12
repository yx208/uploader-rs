#![allow(warnings, warnings)]
use std::path::PathBuf;
use std::sync::Arc;
use tauri::State;
use serde::{Serialize, Deserialize};

mod config;
mod error;
mod models;
mod uploader;
mod utils;

pub use config::TusConfig;
pub use error::{TusError, TusResult};
use uploader::manager::UploadManager;

/// Response type for upload status
#[derive(Debug, Serialize)]
pub struct UploadStatus {
    id: String,
    state: String,
    progress: f64,
    speed: f64,
    file_name: String,
    total_bytes: u64,
    bytes_transferred: u64,
}

impl From<models::upload::Upload> for UploadStatus {
    fn from(upload: models::upload::Upload) -> Self {
        Self {
            id: upload.id,
            state: format!("{:?}", upload.state),
            progress: upload.progress.percentage(),
            speed: upload.progress.speed,
            file_name: upload.filename,
            total_bytes: upload.progress.total_bytes,
            bytes_transferred: upload.progress.bytes_transferred,
        }
    }
}

/// Configuration for initializing the upload system
#[derive(Debug, Deserialize)]
pub struct InitConfig {
    endpoint: String,
    max_concurrent: Option<usize>,
    chunk_size: Option<usize>,
}

/// Manages the upload system state in Tauri
pub struct UploadState {
    manager: Arc<UploadManager>,
}

/// Initialize the upload system
#[tauri::command]
pub async fn init_upload_system(
    config: InitConfig,
    app_handle: tauri::AppHandle,
) -> Result<(), String> {
    let cache_dir = app_handle.path_resolver().app_cache_dir()
        .ok_or("Failed to get cache directory")?;

    let tus_config = TusConfig::new(&config.endpoint)
        .with_state_dir(cache_dir)
        .with_max_concurrent_uploads(config.max_concurrent.unwrap_or(3))
        .with_chunk_size(config.chunk_size.unwrap_or(5 * 1024 * 1024));

    let manager = UploadManager::new(tus_config)
        .await
        .map_err(|e| e.to_string())?;

    app_handle.manage(UploadState { manager });
    Ok(())
}

/// Add a new file to be uploaded
#[tauri::command]
pub async fn add_upload(
    path: String,
    state: State<'_, UploadState>,
) -> Result<String, String> {
    let path = PathBuf::from(path);
    state.manager.add_upload(path)
        .await
        .map_err(|e| e.to_string())
}

/// Start an upload
#[tauri::command]
pub async fn start_upload(
    id: String,
    state: State<'_, UploadState>,
) -> Result<(), String> {
    state.manager.start_upload(&id)
        .await
        .map_err(|e| e.to_string())
}

/// Start all pending uploads
#[tauri::command]
pub async fn start_all_uploads(
    state: State<'_, UploadState>,
) -> Result<(), String> {
    let uploads = state.manager.list_uploads()
        .await
        .map_err(|e| e.to_string())?;

    for upload in uploads {
        if upload.can_start() {
            let _ = state.manager.start_upload(&upload.id).await;
        }
    }
    Ok(())
}

/// Pause an upload
#[tauri::command]
pub async fn pause_upload(
    id: String,
    state: State<'_, UploadState>,
) -> Result<(), String> {
    state.manager.pause_upload(&id)
        .await
        .map_err(|e| e.to_string())
}

/// Pause all active uploads
#[tauri::command]
pub async fn pause_all_uploads(
    state: State<'_, UploadState>,
) -> Result<(), String> {
    let uploads = state.manager.list_uploads()
        .await
        .map_err(|e| e.to_string())?;

    for upload in uploads {
        if upload.is_active() {
            let _ = state.manager.pause_upload(&upload.id).await;
        }
    }
    Ok(())
}

/// Cancel an upload
#[tauri::command]
pub async fn cancel_upload(
    id: String,
    state: State<'_, UploadState>,
) -> Result<(), String> {
    state.manager.cancel_upload(&id)
        .await
        .map_err(|e| e.to_string())
}

/// Cancel all uploads
#[tauri::command]
pub async fn cancel_all_uploads(
    state: State<'_, UploadState>,
) -> Result<(), String> {
    let uploads = state.manager.list_uploads()
        .await
        .map_err(|e| e.to_string())?;

    for upload in uploads {
        let _ = state.manager.cancel_upload(&upload.id).await;
    }
    Ok(())
}

/// Get status of an upload
#[tauri::command]
pub async fn get_upload_status(
    id: String,
    state: State<'_, UploadState>,
) -> Result<UploadStatus, String> {
    state.manager.get_upload_status(&id)
        .await
        .map(UploadStatus::from)
        .map_err(|e| e.to_string())
}

/// List all uploads
#[tauri::command]
pub async fn list_all_uploads(
    state: State<'_, UploadState>,
) -> Result<Vec<UploadStatus>, String> {
    state.manager.list_uploads()
        .await
        .map(|uploads| uploads.into_iter().map(UploadStatus::from).collect())
        .map_err(|e| e.to_string())
}

/// Get number of active uploads
#[tauri::command]
pub async fn get_active_count(
    state: State<'_, UploadState>,
) -> Result<usize, String> {
    Ok(state.manager.get_active_count().await)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::fs::File;
    use tokio::io::AsyncWriteExt;

    async fn setup_test_env() -> (tauri::AppHandle, PathBuf) {
        let app = tauri::test::mock_app();

        // Create test file
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("test.txt");
        let mut file = File::create(&file_path).await.unwrap();
        file.write_all(b"test data").await.unwrap();

        (app, file_path)
    }

    #[tokio::test]
    async fn test_upload_workflow() {
        let (app, file_path) = setup_test_env().await;

        // Initialize system
        init_upload_system(
            InitConfig {
                endpoint: "https://example.com".into(),
                max_concurrent: Some(2),
                chunk_size: Some(1024),
            },
            app.handle(),
        ).await.unwrap();

        let state = app.state::<UploadState>();

        // Add upload
        let id = add_upload(file_path.to_string_lossy().into_owned(), state)
            .await
            .unwrap();

        // Start upload
        start_upload(id.clone(), state).await.unwrap();

        // Get status
        let status = get_upload_status(id.clone(), state).await.unwrap();
        assert_eq!(status.id, id);

        // List uploads
        let uploads = list_all_uploads(state).await.unwrap();
        assert_eq!(uploads.len(), 1);
    }
}