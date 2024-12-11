use std::path::PathBuf;
use std::time::Duration;
use serde::{Serialize, Deserialize};
use crate::error::{TusError, TusResult};

/// Tus上传客户端的配置。这个结构保存了控制上传系统行为的所有设置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TusConfig {
    /// Server configuration
    /// The base URL of the Tus server
    pub endpoint: String,

    /// Additional headers to include in requests
    #[serde(default)]
    pub headers: std::collections::HashMap<String, String>,

    /// Concurrency settings
    /// Maximum number of concurrent uploads
    #[serde(default = "default_max_concurrent_uploads")]
    pub max_concurrent_uploads: usize,

    /// Chunk settings
    /// Size of chunks to upload in bytes
    #[serde(default = "default_chunk_size")]
    pub chunk_size: usize,

    /// Retry settings
    /// Maximum number of retries per chunk
    #[serde(default = "default_max_retries")]
    pub max_retries: u8,

    /// Base delay between retries (will be used with exponential backoff)
    #[serde(default = "default_retry_delay")]
    pub retry_delay: Duration,

    /// Storage settings
    /// Directory to store upload state files
    #[serde(default = "default_state_dir")]
    pub state_dir: PathBuf,

    /// Performance settings
    /// Buffer size for reading files
    #[serde(default = "default_buffer_size")]
    pub buffer_size: usize,
}

// Default configuration values
fn default_max_concurrent_uploads() -> usize { 3 }
fn default_chunk_size() -> usize { 5 * 1024 * 1024 } // 5MB
fn default_max_retries() -> u8 { 3 }
fn default_retry_delay() -> Duration { Duration::from_secs(1) }
fn default_buffer_size() -> usize { 1024 * 1024 } // 1MB
fn default_state_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("tus-uploads")
}

impl Default for TusConfig {
    fn default() -> TusConfig {
        Self {
            endpoint: String::new(),
            headers: std::collections::HashMap::new(),
            max_concurrent_uploads: default_max_concurrent_uploads(),
            chunk_size: default_chunk_size(),
            max_retries: default_max_retries(),
            retry_delay: default_retry_delay(),
            state_dir: default_state_dir(),
            buffer_size: default_buffer_size(),
        }
    }
}

impl TusConfig {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            ..Default::default()
        }
    }

    /// Validates the configuration settings
    pub fn validate(&self) -> TusResult<()> {
        // Validate endpoint
        if self.endpoint.is_empty() {
            return Err(TusError::Config("Endpoint URL cannot be empty".into()));
        }
        if !self.endpoint.starts_with("http://") && !self.endpoint.starts_with("https://") {
            return Err(TusError::Config("Endpoint URL must start with http:// or https://".into()));
        }

        // Validate concurrent uploads
        if self.max_concurrent_uploads == 0 {
            return Err(TusError::Config("Max concurrent uploads must be greater than 0".into()));
        }

        // Validate chunk size
        if self.chunk_size == 0 {
            return Err(TusError::Config("Chunk size must be greater than 0".into()));
        }
        if self.chunk_size > 100 * 1024 * 1024 {
            return Err(TusError::Config("Chunk size cannot be larger than 100MB".into()));
        }

        // Validate buffer size
        if self.buffer_size == 0 {
            return Err(TusError::Config("Buffer size must be greater than 0".into()));
        }
        if self.buffer_size > self.chunk_size {
            return Err(TusError::Config("Buffer size cannot be larger than chunk size".into()));
        }

        Ok(())
    }

    /// Builder method to set custom headers
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Builder method to set maximum concurrent uploads
    pub fn with_max_concurrent_uploads(mut self, max: usize) -> Self {
        self.max_concurrent_uploads = max;
        self
    }

    /// Builder method to set chunk size
    pub fn with_chunk_size(mut self, size: usize) -> Self {
        self.chunk_size = size;
        self
    }

    /// Builder method to set retry settings
    pub fn with_retry_settings(mut self, max_retries: u8, delay: Duration) -> Self {
        self.max_retries = max_retries;
        self.retry_delay = delay;
        self
    }

    /// Builder method to set state directory
    pub fn with_state_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.state_dir = path.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_default_config() {
        let config = TusConfig::default();
        assert_eq!(config.max_concurrent_uploads, 3);
        assert_eq!(config.chunk_size, 5 * 1024 * 1024);
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.retry_delay, Duration::from_secs(1));
    }

    #[test]
    fn test_config_builder() {
        let config = TusConfig::new("https://tus.example.com")
            .with_header("Authorization", "Bearer token")
            .with_max_concurrent_uploads(5)
            .with_chunk_size(10 * 1024 * 1024)
            .with_retry_settings(5, Duration::from_secs(2));

        assert_eq!(config.endpoint, "https://tus.example.com");
        assert_eq!(config.max_concurrent_uploads, 5);
        assert_eq!(config.chunk_size, 10 * 1024 * 1024);
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.retry_delay, Duration::from_secs(2));
        assert_eq!(
            config.headers.get("Authorization").unwrap(),
            "Bearer token"
        );
    }

    #[test]
    fn test_config_validation() {
        // Valid config
        let config = TusConfig::new("https://tus.example.com");
        assert!(config.validate().is_ok());

        // Invalid endpoint
        let config = TusConfig::new("");
        assert!(config.validate().is_err());

        // Invalid chunk size
        let config = TusConfig::new("https://tus.example.com")
            .with_chunk_size(0);
        assert!(config.validate().is_err());

        // Invalid concurrent uploads
        let config = TusConfig::new("https://tus.example.com")
            .with_max_concurrent_uploads(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_serialization() {
        let config = TusConfig::new("https://tus.example.com")
            .with_max_concurrent_uploads(5);

        let serialized = serde_json::to_string(&config).unwrap();
        let deserialized: TusConfig = serde_json::from_str(&serialized).unwrap();

        assert_eq!(config.endpoint, deserialized.endpoint);
        assert_eq!(config.max_concurrent_uploads, deserialized.max_concurrent_uploads);
    }
}

