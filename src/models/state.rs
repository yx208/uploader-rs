use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use crate::config::TusConfig;
use crate::error::{TusError, TusResult};
use crate::models::upload::Upload;

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






    /// 将当前状态保存到磁盘
    async fn persist_state(&self, state: &UploadStateSnapshot) -> TusResult<()> {
        let content = serde_json::to_string_pretty(state)?;
        let temp_file = self.state_file.with_extension("tmp");
        tokio::fs::write(&temp_file, content).await?;

        tokio::fs::rename(&temp_file, &self.state_file).await?;

        Ok(())
    }
}

