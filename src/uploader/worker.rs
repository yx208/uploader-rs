use std::io::SeekFrom;
use std::str::FromStr;
use reqwest::{Client, Request, Url};
use reqwest::header::{HeaderName, HeaderValue};
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::select;
use tokio_util::sync::CancellationToken;
use crate::core::config::TusConfig;
use crate::core::error::{UploadError, UploadResult};
use crate::core::headers;
use crate::core::upload::{Upload, UploadStatus};

pub struct UploadWorker {
    pub upload: Upload,
    client: Client,
    config: TusConfig,
    cancellation_token: CancellationToken,
}

impl UploadWorker {
    pub fn new(config: TusConfig, upload: Upload, token: CancellationToken) -> Self {
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

        let token = self.cancellation_token.clone();
        select! {
            _ = token.cancelled() => {},
            _ = self.start_upload_chunks() => {}
        }

        Ok(())
    }

    /// 执行上传
    /// 参考 Tus 文档：https://tus.io/protocols/resumable-upload#patch
    async fn start_upload_chunks(&mut self) -> UploadResult<()> {
        let file = File::open(&self.upload.file_path).await?;
        let mut reader = BufReader::with_capacity(self.config.buffer_size, file);
        let mut buffer = vec![0u8; self.config.chunk_size];

        let max_retries = self.config.max_retries;
        let mut retry_count = 0;

        loop {
            let offset = self.get_upload_offset().await?;
            if offset >= self.upload.total_bytes {
                self.upload.transition_to(UploadStatus::Completed)?;
                return Ok(());
            }

            reader.seek(SeekFrom::Start(offset)).await?;
            let read_length = reader.read(&mut buffer).await?;
            if read_length == 0 {
                // 如果读不到了，也认为完成
                self.upload.transition_to(UploadStatus::Completed)?;
                return Ok(());
            }

            match self.upload_chunk(&buffer[..read_length], offset).await {
                Ok(_) => {
                    self.upload.progress.update(read_length as u64);
                }
                Err(err) => {
                    retry_count += 1;

                    if retry_count > max_retries {
                        return Err(err);
                    }
                }
            }
        }
    }

    async fn upload_chunk(&mut self, chunk: &[u8], offset: u64) -> UploadResult<()> {
        let url = self.upload.location.as_ref()
            .ok_or_else(|| UploadError::Config("No upload URL available".into()))?;

        let response = self.client
            .patch(url)
            .header(headers::TUS_RESUMABLE, headers::TUS_VERSION)
            .header(headers::UPLOAD_OFFSET, offset.to_string())
            .header(reqwest::header::CONTENT_TYPE, headers::CONTENT_TYPE)
            .body(chunk.to_vec())
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UploadError::Config(format!("Failed to upload chunk: {}", response.status())));
        }

        Ok(())
    }

    async fn build_request(&self) -> UploadResult<Request> {
        let url = Url::parse(&self.config.endpoint)
            .map_err(|_| UploadError::Config("Invalid endpoint".into()))?;

        let mut request = Request::new(reqwest::Method::POST, url);
        let headers = request.headers_mut();

        for (k, v) in self.config.headers.iter() {
            headers.insert(k.parse::<HeaderName>()?, v.parse::<HeaderValue>()?);
        }

        headers.insert(
            HeaderName::from_str(headers::TUS_RESUMABLE)?,
            HeaderValue::from_str(headers::TUS_VERSION)?
        );
        headers.insert(
            HeaderName::from_str(headers::UPLOAD_LENGTH)?,
            HeaderValue::from(self.upload.total_bytes)
        );

        Ok(request)
    }

    /// 再 Tus 服务上创建一个新的上传任务
    /// 参考 Tus 协议文档：https://tus.io/protocols/resumable-upload#creation
    async fn create_upload_in_server(&mut self) -> UploadResult<()> {
        let request = self.build_request().await?;
        let response = self.client.execute(request).await?;

        if !response.status().is_success() {
            return Err(UploadError::Config(format!(
                "Task creation failed, please check the configuration; Code: {}",
                response.status()
            )));
        }

        // 得到资源
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|l| l.to_str().ok())
            .ok_or_else(|| UploadError::Config("No location header in response".to_string()))?;

        self.upload.set_location(location);

        Ok(())
    }

    /// 获取文件再服务端的偏移
    /// 参考 Tus 协议文档：https://tus.io/protocols/resumable-upload#example
    async fn get_upload_offset(&mut self) ->UploadResult<u64> {
        let url = self.upload.location.as_ref()
            .ok_or_else(|| UploadError::Config("No upload URL available".into()))?;

        let response = self.client
            .head(url)
            .header(headers::TUS_RESUMABLE, headers::TUS_VERSION)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(UploadError::Config(format!("Failed to get offset: {}", response.status())));
        }

        let offset = response
            .headers()
            .get(headers::UPLOAD_OFFSET)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .ok_or_else(|| UploadError::Config("Invalid offset in response".to_string()))?;

        Ok(offset)
    }
}

mod tests {
    use super::*;

    fn create_upload() -> Upload {
        let mut file_path = dirs::video_dir().unwrap();
        file_path.push("1086599689-1-209.mp4");
        Upload::new(file_path, 1024 * 1024 * 5).unwrap()
    }

    fn create_worker() -> UploadWorker {
        let config = TusConfig::new("http://127.0.0.1:6440/api/file/tus".to_string());
        let token = CancellationToken::new();
        UploadWorker::new(config, create_upload(), token)
    }

    #[tokio::test]
    async fn test_create_upload_in_server() {
        let mut worker = create_worker();
        worker.create_upload_in_server().await.unwrap();
        assert!(worker.upload.location.is_some());
    }

    #[tokio::test]
    async fn test_upload() {
        let mut worker = create_worker();
        let result = worker.start().await.unwrap();
    }
}
