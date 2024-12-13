use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use crate::core::config::TusConfig;
use crate::core::error::UploadResult;
use crate::core::upload::Upload;

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadStateSnapshot {
    /// 格式变动兼容
    version: u8,

    /// 上传任务映射
    uploads: HashMap<String, Upload>,

    /// 上传配置
    config: TusConfig,
}

impl UploadStateSnapshot {
    pub fn new(config: TusConfig) -> Self {
        Self {
            version: 1,
            config,
            uploads: HashMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UploadStateManager {
    /// 状态
    state: Arc<RwLock<UploadStateSnapshot>>,

    /// 文件保存路径
    state_file: PathBuf,
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
        })
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
