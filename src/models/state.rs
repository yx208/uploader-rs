use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use crate::config::TusConfig;
use crate::error::{TusError, TusResult};
use crate::models::upload::{Upload, UploadState};

/// 表示系统中所有上传的完整状态
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadStateSnapshot {
    /// 状态格式的版本，用于将来的兼容性
    version: u32,

    /// 上传ID到上传实例的映射
    uploads: HashMap<String, Upload>,

    /// 用于上传的配置
    config: TusConfig,
}

/// UploadManager 处理系统中所有上传的状态
/// 提供对上传状态的线程安全访问并处理持久性
#[derive(Debug, Clone)]
pub struct UploadManager {
    /// 上传状态
    state: Arc<RwLock<UploadStateSnapshot>>,

    /// 状态保存路径
    state_file: PathBuf,
}

impl UploadStateSnapshot {
    pub fn new(config: TusConfig) -> Self {
        Self {
            config,
            version: 1,
            uploads: HashMap::new(),
        }
    }
}

impl UploadManager {
    pub async fn new(config: TusConfig) -> TusResult<Self> {
        tokio::fs::create_dir_all(&config.state_dir).await?;

        let state_file = config.state_dir.join("upload_state.json");
        let state = if state_file.exists() {
            // load
            let content = tokio::fs::read_to_string(&state_file).await?;
            serde_json::from_str(&content)?
        } else {
            UploadStateSnapshot::new(config)
        };

        Ok(Self {
            state_file,
            state: Arc::new(RwLock::new(state)),
        })
    }

    pub async fn add_upload(&self, upload: Upload) -> TusResult<()> {
        let mut state = self.state.write().await;
        state.uploads.insert(upload.id.clone(), upload);
        self.persist_state(&state).await?;

        Ok(())
    }

    pub async fn get_upload(&self, id: &str) -> TusResult<Upload> {
        let state = self.state.read().await;
        state.uploads.get(id)
            .cloned()
            .ok_or_else(|| TusError::UploadNotFound(id.to_string()))
    }

    pub async fn remove_upload(&self, id: &str) -> TusResult<()> {
        let mut state = self.state.write().await;
        if state.uploads.remove(id).is_none() {
            return Err(TusError::UploadNotFound(id.to_string()));
        }
        self.persist_state(&state).await?;

        Ok(())
    }

    pub async fn update_upload(&self, upload: Upload) -> TusResult<()> {
        let mut state = self.state.write().await;
        if !state.uploads.contains_key(&upload.id) {
            return Err(TusError::UploadNotFound(upload.id));
        }
        state.uploads.insert(upload.id.clone(), upload);
        self.persist_state(&state).await?;

        Ok(())
    }

    pub async fn get_uploads_by_state(&self, state: UploadState) -> Vec<Upload> {
        let state_snapshot = self.state.read().await;
        state_snapshot.uploads.values()
            .filter(|upload| upload.state == state)
            .cloned()
            .collect()
    }

    pub async fn get_resumable_uploads(&self) -> Vec<Upload> {
        let state_snapshot = self.state.read().await;
        state_snapshot.uploads.values()
            .filter(|upload| upload.can_start())
            .cloned()
            .collect()
    }

    pub async fn active_upload_count(&self) -> usize {
        let state_snapshot = self.state.read().await;
        state_snapshot.uploads.values()
            .filter(|upload| upload.is_active())
            .count()
    }

    /// 将当前状态保存到磁盘
    async fn persist_state(&self, state: &UploadStateSnapshot) -> TusResult<()> {
        let content = serde_json::to_string_pretty(state)?;
        let temp_file = self.state_file.with_extension("tmp");
        tokio::fs::create_dir_all(temp_file.parent().unwrap()).await?;
        tokio::fs::write(&temp_file, content).await?;
        tokio::fs::rename(&temp_file, &self.state_file).await?;

        Ok(())
    }

    /// 开放 api
    pub async fn save_state(&self) -> TusResult<()> {
        let state = self.state.read().await;
        self.persist_state(&state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    async fn create_test_manager() -> TusResult<UploadManager> {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = TusConfig::new("https://example.com")
            .with_state_dir(temp_dir.path().to_owned());
        UploadManager::new(config).await
    }

    #[tokio::test]
    async fn test_upload_lifecycle_management() -> TusResult<()> {
        let manager = create_test_manager().await?;

        // Create a test upload
        let temp_file = tempfile::NamedTempFile::new()?;
        let upload = Upload::new(temp_file.path().to_owned(), 1024)?;
        let upload_id = upload.id.clone();

        // Test adding upload
        manager.add_upload(upload.clone()).await?;

        // Test retrieving upload
        let retrieved = manager.get_upload(&upload_id).await?;
        assert_eq!(retrieved.id, upload_id);

        // Test updating upload
        let mut updated = retrieved;
        updated.transition_to(UploadState::Active)?;
        manager.update_upload(updated.clone()).await?;

        let retrieved = manager.get_upload(&upload_id).await?;
        assert_eq!(retrieved.state, UploadState::Active);

        // Test removing upload
        manager.remove_upload(&upload_id).await?;
        assert!(manager.get_upload(&upload_id).await.is_err());

        Ok(())
    }

    #[tokio::test]
    async fn test_state_persistence() -> TusResult<()> {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = TusConfig::new("https://example.com")
            .with_state_dir(temp_dir.path().to_owned());

        // Create manager and add upload
        let manager = UploadManager::new(config.clone()).await?;
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let upload = Upload::new(temp_file.path().to_owned(), 1024)?;
        let upload_id = upload.id.clone();

        manager.add_upload(upload).await?;

        // Create new manager instance (simulating program restart)
        let new_manager = UploadManager::new(config).await?;

        // Verify upload state was restored
        let retrieved = new_manager.get_upload(&upload_id).await?;
        assert_eq!(retrieved.id, upload_id);

        Ok(())
    }

    #[tokio::test]
    async fn test_concurrent_access() -> TusResult<()> {
        let manager = create_test_manager().await?;
        let manager_clone = manager.clone();

        // Create test upload
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let upload = Upload::new(temp_file.path().to_owned(), 1024)?;
        let upload_id = upload.id.clone();

        manager.add_upload(upload).await?;

        // Spawn task to update upload
        let inner_upload_id = upload_id.clone();
        let update_task = tokio::spawn(async move {
            let mut upload = manager_clone.get_upload(&inner_upload_id).await?;
            upload.transition_to(UploadState::Active)?;
            manager_clone.update_upload(upload).await
        });

        // Try to read while update is in progress
        sleep(Duration::from_millis(10)).await;
        let retrieved = manager.get_upload(&upload_id).await?;

        // Wait for update to complete
        update_task.await.unwrap()?;

        // Verify states
        assert!(retrieved.state == UploadState::Pending || retrieved.state == UploadState::Active);
        let final_state = manager.get_upload(&upload_id).await?.state;
        assert_eq!(final_state, UploadState::Active);

        Ok(())
    }
}
