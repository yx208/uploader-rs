use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::select;
use tokio::sync::{RwLock, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use crate::core::config::TusConfig;
use crate::core::error::UploadResult;
use crate::core::state::UploadStateManager;
use crate::core::upload::{Upload, UploadStatus};
use crate::uploader::worker::UploadWorker;

struct ActiveUpload {
    handle: JoinHandle<Upload>,

    /// child token
    cancellation_token: CancellationToken
}

pub struct UploadManager {
    // 所有的 upload
    upload_state: UploadStateManager,

    // 上传配置
    config: TusConfig,

    // 正在上传的 upload
    active_uploads: Arc<RwLock<HashMap<String, ActiveUpload>>>,

    // 非 pending 状态的 upload 放这里
    shelved_uploads: Arc<RwLock<Vec<Upload>>>,

    // 并发锁
    semaphore: Arc<Semaphore>,

    // token
    cancellation_token: CancellationToken
}

impl UploadManager {
    pub async fn new(config: TusConfig) -> UploadResult<Self> {
        let upload_state = UploadStateManager::new(config.clone()).await?;
        let active_uploads = Arc::new(RwLock::new(HashMap::new()));
        let semaphore = Arc::new(Semaphore::new(config.max_concurrent));
        let cancellation_token = CancellationToken::new();
        let shelved_uploads = Arc::new(RwLock::new(Vec::new()));

        Ok(Self {
            config,
            upload_state,
            active_uploads,
            semaphore,
            cancellation_token,
            shelved_uploads,
        })
    }

    /// 开是运行循环执行任务
    pub async fn run(&self) {
        let semaphore = self.semaphore.clone();
        loop {
            // 获取信号量
            let permit = semaphore.clone().acquire_owned().await.unwrap();

            // 创建 worker
            let upload = self.upload_state.pop().await;
            let upload_id = upload.id.clone();
            let mut worker = UploadWorker::new(self.config.clone(), upload, self.cancellation_token.child_token());

            // 执行 upload
            let child_token = self.cancellation_token.child_token();
            let cancellation_token = child_token.clone();
            let handle = tokio::spawn(async move {
                let future = worker.start();

                select! {
                    _ = cancellation_token.cancelled() => {},
                    result = future => {
                        match result {
                            Ok(res) => {

                            }
                            Err(err) => {

                            }
                        }
                    }
                }

                drop(permit);
                worker.upload
            });

            // 添加任务列表
            {
                let mut active_guard = self.active_uploads.write().await;
                active_guard.insert(upload_id, ActiveUpload {
                    handle,
                    cancellation_token: child_token,
                });
            }
        }
    }

    /// 创建一个新的 upload
    /// 新的 upload 最初状态是 pending，添加到 upload_state 中
    pub async fn add_upload(&self, file_path: PathBuf) -> UploadResult<String> {
        let upload = Upload::new(file_path, self.config.chunk_size)?;
        let upload_id = upload.id.clone();
        self.upload_state.push(upload).await?;

        Ok(upload_id)
    }

    /// 暂停 upload
    /// 从 active 中移除，添加到 shelved 中
    pub async fn pause_upload(&self, id: String) -> UploadResult<()> {
        let mut active_guard = self.active_uploads.write().await;
        if let Some(active_upload) = active_guard.remove(&id) {
            active_upload.cancellation_token.cancel();
            match active_upload.handle.await {
                Ok(mut upload) => {
                    if let Ok(_) = upload.transition_to(UploadStatus::Paused) {
                        let mut shelved_guard = self.shelved_uploads.write().await;
                        shelved_guard.push(upload);
                    }
                }
                Err(err) => {
                    println!("{}", err);
                }
            };
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::{join, select};
    use tokio_util::sync::CancellationToken;

    async fn task1(token: CancellationToken) {
        let timer = tokio::time::sleep(tokio::time::Duration::from_secs(3));

        let token_clone = token.clone();
        tokio::spawn(async move {
            let timer = tokio::time::sleep(tokio::time::Duration::from_secs(1));
            token_clone.cancel();
        });

        select! {
            r = timer => {
                println!("timer expired");
            }
            r = token.cancelled() => {
                println!("cancelled");
            }
        };
    }

    async fn task2(token: CancellationToken) {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    }

    #[tokio::test]
    async fn test_select() {
        // let f1 = task1();
        // let f2 = task2();

        let token = CancellationToken::new();

        let child_token = token.child_token();
        tokio::spawn(async move {
            task1(child_token).await;
        });

        let handle = tokio::spawn(async move {
            select! {
                r = token.cancelled() => {
                    println!("parent cancelled");
                }
                r = tokio::time::sleep(tokio::time::Duration::from_secs(2)) => {
                    println!("done");
                }
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        join!(handle);
    }

    #[tokio::test]
    async fn test_create() {
        let config = TusConfig::new("http://127.0.0.1:6440/api/file/tus".to_string());
        let upload_manager = UploadManager::new(config).await.unwrap();
    }
}
