use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{watch, RwLock, Semaphore};
use tokio::task::JoinHandle;

use crate::config::TusConfig;
use crate::error::{TusError, TusResult};
use crate::models::upload::{Upload, UploadState};
use crate::models::state::UploadManager as StateManager;
use crate::uploader::worker::UploadWorker;

struct ActiveUpload {
    upload: Upload,
    handle: JoinHandle<TusResult<Upload>>,
    stop_tx: watch::Sender<bool>,
}

pub struct UploadManager {
    config: TusConfig,
    state_manager: Arc<StateManager>,
    active_uploads: Arc<RwLock<HashMap<String, ActiveUpload>>>,
    semaphore: Arc<Semaphore>,
}

impl UploadManager {
    pub async fn new(config: TusConfig) -> TusResult<Arc<Self>> {
        let state_manager = Arc::new(StateManager::new(config.clone()).await?);
        let active_uploads = Arc::new(RwLock::new(HashMap::new()));
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent_uploads));

        Ok(Arc::new(Self {
            config,
            state_manager,
            active_uploads,
            semaphore,
        }))
    }

    pub async fn add_upload(&self, file_path: PathBuf) -> TusResult<String> {
        let upload = Upload::new(file_path, self.config.chunk_size)?;
        let id = upload.id.clone();
        self.state_manager.add_upload(upload).await?;
        Ok(id)
    }

    pub async fn start_upload(&self, id: &str) -> TusResult<()> {
        let upload = self.state_manager.get_upload(id).await?;
        if !upload.can_start() {
            return Err(TusError::InvalidState("Upload cannot be started".into()));
        }

        // Clone what we need for the task
        let id = upload.id.clone();
        let config = self.config.clone();
        let state_manager = self.state_manager.clone();
        let active_uploads = self.active_uploads.clone();
        let semaphore = self.semaphore.clone();

        // Spawn the upload task
        tokio::spawn(async move {
            // Acquire semaphore permit
            let _permit = semaphore.acquire().await.unwrap();

            // Setup stop channel
            let (stop_tx, stop_rx) = watch::channel(false);

            // Create and start worker
            let mut worker = UploadWorker::new(config, upload, Some(stop_rx));
            let clone_upload = worker.upload.clone();

            let handle = tokio::spawn(async move {
                let result = worker.start().await;
                if let Ok(ref upload) = result {
                    let _ = state_manager.update_upload(upload.clone()).await;
                }
                result
            });

            // Store active upload
            let active_upload = ActiveUpload {
                upload: clone_upload,
                handle,
                stop_tx,
            };
            active_uploads.write().await.insert(id.clone(), active_upload);

            // The permit will be released when this task ends
        });

        Ok(())
    }

    pub async fn pause_upload(&self, id: &str) -> TusResult<()> {
        let mut active_uploads = self.active_uploads.write().await;
        if let Some(active) = active_uploads.get(id) {
            let _ = active.stop_tx.send(true);
            Ok(())
        } else {
            Err(TusError::UploadNotFound(id.to_string()))
        }
    }

    pub async fn cancel_upload(&self, id: &str) -> TusResult<()> {
        let mut active_uploads = self.active_uploads.write().await;
        if let Some(active) = active_uploads.remove(id) {
            // Signal stop to worker
            let _ = active.stop_tx.send(true);
            // Abort the task
            active.handle.abort();
            // Update state
            let mut upload = active.upload;
            upload.transition_to(UploadState::Cancelled)?;
            self.state_manager.update_upload(upload).await?;
            Ok(())
        } else {
            Err(TusError::UploadNotFound(id.to_string()))
        }
    }

    pub async fn get_upload_status(&self, id: &str) -> TusResult<Upload> {
        self.state_manager.get_upload(id).await
    }

    pub async fn get_active_count(&self) -> usize {
        self.active_uploads.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn create_test_file() -> PathBuf {
        let dir = dirs::video_dir().unwrap();
        let file_path = dir.join("1148739452-1-30120.mp4");
        file_path
    }

    async fn create_test_file2() -> PathBuf {
        let dir = dirs::video_dir().unwrap();
        let file_path = dir.join("Castle-in-the-Sky.mp4");
        file_path
    }

    async fn create_test_file3() -> PathBuf {
        let dir = dirs::video_dir().unwrap();
        let file_path = dir.join("beijing.mp4");
        file_path
    }

    async fn create_manager() -> TusResult<Arc<UploadManager>> {
        let config = TusConfig::new("http://127.0.0.1:6440");
        UploadManager::new(config).await
    }

    #[tokio::test]
    async fn test_concurrent_uploads() -> TusResult<()> {
        let file_path1 = create_test_file().await;
        let file_path2 = create_test_file2().await;
        let file_path3 = create_test_file3().await;

        let manager = create_manager().await?;

        // Add and start three uploads
        let id1 = manager.add_upload(file_path1).await?;
        let id2 = manager.add_upload(file_path2).await?;
        let id3 = manager.add_upload(file_path3).await?;

        manager.start_upload(&id1).await?;
        manager.start_upload(&id2).await?;
        manager.start_upload(&id3).await?;

        // Give some time for tasks to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify that only two uploads are active
        assert_eq!(manager.get_active_count().await, 2);

        // Cancel one upload
        manager.cancel_upload(&id1).await?;

        // Give some time for the third upload to start
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify that we still have two active uploads
        assert_eq!(manager.get_active_count().await, 2);

        Ok(())
    }

    #[tokio::test]
    async fn test_upload_lifecycle() -> TusResult<()> {
        let file_path = create_test_file().await;
        let manager = create_manager().await?;

        // Test Pending state
        let id = manager.add_upload(file_path).await?;
        assert_eq!(manager.get_upload_status(&id).await?.state, UploadState::Pending);

        // Test Active state
        manager.start_upload(&id).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Test Paused state
        manager.pause_upload(&id).await?;
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        assert_eq!(manager.get_upload_status(&id).await?.state, UploadState::Paused);

        // Test Cancelled state
        manager.cancel_upload(&id).await?;
        assert_eq!(manager.get_upload_status(&id).await?.state, UploadState::Cancelled);

        Ok(())
    }
}