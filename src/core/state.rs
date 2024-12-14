use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, RwLock};
use crate::core::config::TusConfig;
use crate::core::error::{UploadError, UploadResult};
use crate::core::upload::Upload;

#[derive(Debug, Serialize, Deserialize)]
struct UploadStateSnapshot {
    /// 格式变动兼容
    version: u8,

    /// pending 状态任务
    uploads: VecDeque<Upload>,

    /// 上传配置
    config: TusConfig,
}

impl UploadStateSnapshot {
    pub fn new(config: TusConfig) -> Self {
        Self {
            version: 1,
            config,
            uploads: VecDeque::new(),
        }
    }
}

#[derive(Debug)]
pub struct UploadStateManager {
    /// 状态
    state: Arc<RwLock<UploadStateSnapshot>>,

    /// 文件保存路径
    state_file: PathBuf,

    /// 任务添加通知
    notify: Notify,
}

impl UploadStateManager {
    pub async fn new(config: TusConfig) -> UploadResult<Self> {
        /// 创建这个目录
        if !config.state_dir.exists() {
            tokio::fs::create_dir_all(&config.state_dir).await?;
        }

        let state_file = config.state_dir.join("upload-state.json");
        let state_snapshot = if state_file.exists() {
            // load
            let content = tokio::fs::read_to_string(&state_file).await?;
            serde_json::from_str(&content)?
        } else {
            // init
            UploadStateSnapshot::new(config)
        };

        Ok(Self {
            state_file,
            state: Arc::new(RwLock::new(state_snapshot)),
            notify: Notify::new(),
        })
    }

    pub async fn push(&self, upload: Upload) -> UploadResult<()> {
        let mut state = self.state.write().await;
        state.uploads.push_back(upload);
        self.notify.notify_waiters();

        self.persist_state(&state).await?;

        Ok(())
    }

    pub async fn remove(&self, id: String) {
        let mut state = self.state.write().await;
        state.uploads.retain(|upload| upload.id == id);
    }

    pub async fn get_upload(&self, id: &str) -> UploadResult<Upload> {
        let state = self.state.read().await;
        state.uploads
            .iter()
            .find(|u| u.id == id)
            .cloned()
            .ok_or_else(|| UploadError::UploadNotFound(id.to_string()))
    }

    /// 弹出最前面的 upload
    /// 如果没有 upload 则等待 push 后的 notify
    pub async fn pop(&self) -> Upload {
        loop {
            let mut state = self.state.write().await;
            if let Some(upload) = state.uploads.pop_front() {
                return upload;
            }
            drop(state);

            self.notify.notified().await;
        }
    }

    /// 持久化状态
    async fn persist_state(&self, state: &UploadStateSnapshot) -> UploadResult<()> {
        let content = serde_json::to_string_pretty(state)?;
        // 安全写入
        let temp_file = self.state_file.with_extension("tmp");
        // 在 new 中已校验过文件夹
        tokio::fs::write(&temp_file, content).await?;
        tokio::fs::rename(&temp_file, &self.state_file).await?;

        Ok(())
    }

    /// 提供外部调用
    pub async fn save_state(&self) -> UploadResult<()> {
        let state = self.state.read().await;
        self.persist_state(&state).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_queue() {
        let config = TusConfig::default();

        let manager = UploadStateManager::new(config).await.unwrap();
        let manager = Arc::new(manager);

        let mut file_path = dirs::video_dir().unwrap();
        file_path.push("1086599689-1-209.mp4");
        let upload = Upload::new(file_path, 1024 * 1024 * 5).unwrap();
        let upload_id = upload.id.clone();

        let manager_clone = manager.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            manager_clone.push(upload).await.unwrap();
        });

        let added_upload = manager.pop().await;
        assert_eq!(upload_id, added_upload.id);
    }
}
