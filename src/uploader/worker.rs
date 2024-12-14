use reqwest::{Client, Request, Url};
use tokio_util::sync::CancellationToken;
use wiremock::http::{HeaderName, HeaderValue};
use crate::core::config::TusConfig;
use crate::core::error::{UploadError, UploadResult};
use crate::core::upload::{Upload, UploadStatus};

pub struct UploadWorker {
    pub upload: Upload,
    client: Client,
    config: TusConfig,
    cancellation_token: Option<CancellationToken>,
}

impl UploadWorker {
    pub fn new(config: TusConfig, upload: Upload, token: Option<CancellationToken>) -> Self {
        Self {
            config,
            upload,
            client: Client::new(),
            cancellation_token: token,
        }
    }

    /// 开始以及检查配置
    pub async fn start(&mut self) -> UploadResult<()> {
        if !self.upload.can_start() {
            return Err(UploadError::InvalidState("Upload cannot be started in current state".into()));
        }

        self.upload.transition_to(UploadStatus::Active)?;

        if self.upload.location.is_none() {
            self.create_upload_in_server().await?;
        }

        self.start_upload_chunks().await?;

        Ok(())
    }

    /// 执行上传
    async fn start_upload_chunks(&mut self) -> UploadResult<()> {
        Ok(())
    }

    async fn build_request(&self) -> UploadResult<Request> {
        let url = Url::parse(&self.config.endpoint)
            .map_err(|_| UploadError::ConfigError("Invalid endpoint".into()))?;

        let mut request = Request::new(reqwest::Method::POST, url);
        let headers = request.headers_mut();

        for (k, v) in self.config.headers.iter() {
            headers.insert(k.parse::<HeaderName>()?, v.parse::<HeaderValue>()?);
        }

        headers.insert(HeaderName::from_bytes(b"Tus-Resumable")?, HeaderValue::from_bytes(b"1.0.0")?);
        headers.insert(HeaderName::from_bytes(b"Upload-Length")?, HeaderValue::from(self.upload.total_bytes));

        Ok(request)
    }

    /// 再 Tus 服务上创建一个新的上传任务
    /// 参考 Tus 协议文档：https://tus.io/protocols/resumable-upload#creation
    async fn create_upload_in_server(&mut self) -> UploadResult<()> {
        let request = self.build_request().await?;
        let response = self.client.execute(request).await?;

        if !response.status().is_success() {
            return Err(UploadError::ConfigError(format!(
                "Task creation failed, please check the configuration; Code: {}",
                response.status()
            )));
        }

        // 得到资源
        let location = response
            .headers()
            .get("Location")
            .and_then(|l| l.to_str().ok())
            .ok_or_else(|| UploadError::ConfigError("No location header in response".to_owned()))?;

        self.upload.set_location(location);

        Ok(())
    }
}

mod tests {
    use super::*;

    fn create_upload() -> Upload {
        let mut file_path = dirs::video_dir().unwrap();
        file_path.push("1086599689-1-209.mp4");
        Upload::new(file_path, 1024 * 1024 * 5).unwrap()
    }

    #[tokio::test]
    async fn test_create_upload_in_server() {
        let config = TusConfig::new("http://127.0.0.1:6440/api/file/tus".to_string());
        let mut worker = UploadWorker::new(config, create_upload(), None);
        worker.create_upload_in_server().await.unwrap();
        assert!(worker.upload.location.is_some());
    }
}