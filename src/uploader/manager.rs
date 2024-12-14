use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, Semaphore};
use crate::core::config::TusConfig;
use crate::core::error::UploadResult;
use crate::core::state::UploadStateManager;
use crate::core::upload::Upload;

pub struct UploadManager {
    upload_state: UploadStateManager,
    config: TusConfig,
    active_uploads: Arc<RwLock<HashMap<String, Upload>>>,
    semaphore: Arc<Semaphore>,
}

impl UploadManager {
    pub async fn new(config: TusConfig) -> UploadResult<Self> {
        let upload_state = UploadStateManager::new(config.clone()).await?;
        let active_uploads = Arc::new(RwLock::new(HashMap::new()));
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));

        Ok(Self {
            config,
            upload_state,
            active_uploads,
            semaphore,
        })
    }

    pub async fn add_upload(&self) {

    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_create() {
        let config = TusConfig::new("http://127.0.0.1:6440/api/file/tus".to_string());
        let upload_manager = UploadManager::new(config).await.unwrap();
    }
}
