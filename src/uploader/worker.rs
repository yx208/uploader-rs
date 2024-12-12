use std::io::SeekFrom;
use std::time::Duration;
use reqwest::{Client, Request};
use reqwest::header;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, BufReader};
use tokio::sync::watch::Receiver;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use crate::config::TusConfig;
use crate::error::{TusError, TusResult};
use crate::models::upload::{Upload, UploadState};

const TUS_VERSION: &str = "1.0.0";
const TUS_RESUMABLE: &str = "Tus-Resumable";
const TUS_VERSION_HEADER: &str = "Tus-Version";
const UPLOAD_LENGTH_HEADER: &str = "Upload-Length";
const UPLOAD_OFFSET_HEADER: &str = "Upload-Offset";
const CONTENT_TYPE: &str = "application/offset+octet-stream";

/// 使用Tus协议处理单个文件的上传过程
pub struct UploadWorker {
    client: Client,
    config: TusConfig,
    pub upload: Upload,
    stop_signal: Option<Receiver<bool>>
}

impl UploadWorker {
    pub fn new(config: TusConfig, upload: Upload, stop_signal: Option<Receiver<bool>>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();

        Self {
            client,
            config,
            upload,
            stop_signal
        }
    }

    /// 开始或恢复上传过程
    pub async fn start(&mut self) -> TusResult<Upload> {
        if !self.upload.can_start() {
            return Err(TusError::InvalidState("Upload cannot be started in current state".into()));
        }

        self.upload.transition_to(UploadState::Active)?;

        if self.upload.upload_url.is_none() {
            self.create_upload().await?;
        }

        self.upload_chunks().await?;

        Ok(self.upload.clone())
    }

    /// 在 Tus 服务器上创建一个新的上传
    async fn create_upload(&mut self) -> TusResult<()> {
        let response = self.client
            .post(&self.config.endpoint)
            .header(TUS_RESUMABLE, TUS_VERSION)
            .header(UPLOAD_LENGTH_HEADER, self.upload.progress.total_bytes.to_string())
            .headers(self.create_metadata_headers())
            .send()
            .await
            .map_err(|err| TusError::NetworkError(err))?;

        if !response.status().is_success() {
            return Err(TusError::Config(format!(
                "Failed to create upload: {}",
                response.status()
            )))
        }

        let location = response
            .headers()
            .get("location")
            .and_then(|h| h.to_str().ok())
            .ok_or_else(|| TusError::Config("No location header in response".into()))?;
        self.upload.set_upload_url(location);

        Ok(())
    }

    async fn upload_chunks(&mut self) -> TusResult<()> {
        let file = File::open(&self.upload.file_path).await?;
        let mut reader = BufReader::with_capacity(self.config.buffer_size, file);
        let mut buffer = vec![0u8; self.config.chunk_size];

        loop {
            // 检查停止
            if let Some(ref receiver) = self.stop_signal {
                if *receiver.borrow() {
                    self.upload.transition_to(UploadState::Paused)?;
                    return Ok(());
                }
            }

            // 从服务器获取当前偏移量
            let offset = self.get_offset().await?;
            if offset >= self.upload.progress.total_bytes {
                self.upload.transition_to(UploadState::Completed)?;
                return Ok(());
            }

            // 偏移
            reader.seek(SeekFrom::Start(offset)).await?;

            // Read chunk
            let n = reader.read(&mut buffer).await?;
            if n == 0 {
                break;
            }

            // 上传
            let mut retries = 0;
            loop {
                match self.upload_chunk(&buffer[..n], offset).await {
                    Ok(_) => {
                        self.upload.update_progress(n as u64, true);
                        break;
                    }
                    Err(err) => {
                        retries += 1;
                        if retries >= self.config.max_retries {
                            return Err(err);
                        }
                        tokio::time::sleep(self.get_retry_delay(retries)).await;
                    }
                }
            }
        }

        Ok(())
    }

    async fn upload_chunk(&self, chunk: &[u8], offset: u64) -> TusResult<()> {
        let url = self.upload.upload_url.as_ref()
            .ok_or_else(|| TusError::Config("No upload URL available".into()))?;

        let response = self.client
            .patch(url)
            .header(TUS_RESUMABLE, TUS_VERSION)
            .header(UPLOAD_OFFSET_HEADER, offset.to_string())
            .header(header::CONTENT_TYPE, CONTENT_TYPE)
            .body(chunk.to_vec())
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(TusError::Config(format!(
                "Failed to upload chunk: {}",
                response.status()
            )));
        }

        Ok(())
    }

    /// 获取当前从服务器上传的偏移量
    async fn get_offset(&mut self) -> TusResult<u64> {
        let url = self.upload.upload_url.as_ref()
            .ok_or_else(|| TusError::Config("No upload URL available".into()))?;
        println!("{}", &url);
        let response = self.client
            .head(url)
            .header(TUS_RESUMABLE, TUS_VERSION)
            .send()
            .await
            .map_err(|err| {
                TusError::NetworkError(err)
            })?;

        if !response.status().is_success() {
            return Err(TusError::Config(format!(
                "Failed to get offset: {}",
                response.status()
            )));
        }

        let offset = response
            .headers()
            .get(UPLOAD_OFFSET_HEADER)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<u64>().ok())
            .ok_or_else(|| TusError::Config("Invalid offset in response".into()))?;

        Ok(offset)
    }

    /// 从上传元数据创建元数据标头
    fn create_metadata_headers(&self) -> header::HeaderMap {
        let mut headers = header::HeaderMap::new();

        // 将元数据转换为Tus格式
        let metadata: String = self.upload.metadata.iter()
            .map(|(k, v)| {
                let encoded = BASE64.encode(v);
                format!("{} {}", k, encoded)
            })
            .collect::<Vec<_>>()
            .join(".");

        if !metadata.is_empty() {
            headers.insert(
                "Upload-Metadata",
                header::HeaderValue::from_str(&metadata).unwrap_or(header::HeaderValue::from_static(""))
            );
        }

        headers
    }

    /// 使用指数退避计算重试延迟
    fn get_retry_delay(&self, retry: u8) -> Duration {
        let base = self.config.retry_delay.as_secs_f64();
        let delay = base * (2_f64.powi(retry as i32 - 1));
        Duration::from_secs_f64(delay.min(30.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{MockServer, Mock, ResponseTemplate};
    use wiremock::matchers::{method, path, header};

    async fn create_test_upload() -> (UploadWorker, tempfile::NamedTempFile, MockServer) {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let vec = vec![0u8; 10 * 1024 * 1024];
        tokio::fs::write(&temp_file, &vec).await.unwrap();

        let mock_server = MockServer::start().await;
        let config = TusConfig::new(&mock_server.uri());
        let upload = Upload::new(temp_file.path().to_owned(), 1024).unwrap();
        let worker = UploadWorker::new(config, upload, None);

        (worker, temp_file, mock_server)
    }

    #[tokio::test]
    async fn test_create_upload() {
        let (mut worker, _temp_file, mock_server) = create_test_upload().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .and(header(TUS_RESUMABLE, TUS_VERSION))
            .respond_with(ResponseTemplate::new(201)
                .insert_header("location", "/files/test123"))
            .mount(&mock_server)
            .await;

        assert!(worker.create_upload().await.is_ok());
        assert_eq!(worker.upload.upload_url, Some("/files/test123".to_string()));
    }

    #[tokio::test]
    async fn test_get_offset() {
        let (mut worker, _temp_file, mock_server) = create_test_upload().await;
        worker.upload.set_upload_url(format!("{}/files/test123", mock_server.uri()));

        Mock::given(method("HEAD"))
            .and(path("/files/test123"))
            .and(header(TUS_RESUMABLE, TUS_VERSION))
            .respond_with(ResponseTemplate::new(200)
                .insert_header(UPLOAD_OFFSET_HEADER, "100"))
            .mount(&mock_server)
            .await;

        assert_eq!(worker.get_offset().await.unwrap(), 100);
    }
}
