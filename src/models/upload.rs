use std::collections::HashMap;
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::{TusError, TusResult};

/// Represents the current state of an upload
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UploadState {
    /// Upload is created but not yet started
    Pending,

    /// Upload is actively transferring data
    Active,

    /// Upload is temporarily stopped but can be resumed
    Paused,

    /// Upload has been permanently stopped
    Cancelled,

    /// Upload has completed successfully
    Completed,

    /// Upload encountered an error
    Failed,
}

impl UploadState {
    /// 检查状态是否可以转换到目标状态
    pub fn can_transition_to(&self, target: UploadState) -> bool {
        use UploadState::*;

        match (*self, target) {
            // From Pending
            (Pending, Active) => true,
            (Pending, Cancelled) => true,

            // From Active
            (Active, Paused) => true,
            (Active, Cancelled) => true,
            (Active, Completed) => true,
            (Active, Failed) => true,

            // From Paused
            (Paused, Active) => true,
            (Paused, Cancelled) => true,

            // Terminal states can't transition
            (Completed, _) => false,
            (Cancelled, _) => false,
            (Failed, _) => false,

            // All other transitions are invalid
            _ => false,
        }
    }
}

/// Represents the progress of an upload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadProgress {
    /// Number of bytes transferred
    pub bytes_transferred: u64,

    /// Total size of the file in bytes
    pub total_bytes: u64,

    /// Current transfer speed in bytes per second
    pub speed: f64,

    /// Number of chunks successfully uploaded
    pub chunks_completed: u32,

    /// Total number of chunks
    pub total_chunks: u32,

    /// Last error message if any
    pub last_error: Option<String>,

    /// Timestamp of the last progress update
    pub last_updated: DateTime<Utc>,
}

impl UploadProgress {
    pub fn new(total_bytes: u64, chunk_size: usize) -> Self {
        let total_chunks = ((total_bytes as f64) / (chunk_size as f64)).ceil() as u32;

        Self {
            bytes_transferred: 0,
            total_bytes,
            speed: 0.0,
            chunks_completed: 0,
            total_chunks,
            last_error: None,
            last_updated: Utc::now(),
        }
    }

    /// 更新进度
    pub fn update(&mut self, new_bytes: u64, chunk_completed: bool) {
        let now = Utc::now();
        let duration = (now - self.last_updated).num_milliseconds() as f64 / 1000.0;

        if duration > 0.0 {
            // Calculate speed using simple moving average
            let instant_speed = (new_bytes as f64) / duration;
            self.speed = if self.speed == 0.0 {
                instant_speed
            } else {
                (self.speed * 0.7) + (instant_speed * 0.3)
            };
        }

        self.bytes_transferred += new_bytes;
        if chunk_completed {
            self.chunks_completed += 1;
        }
        self.last_updated = now;
    }

    /// 返回百分比进度
    pub fn percentage(&self) -> f64 {
        if self.total_bytes == 0 {
            0.0
        } else {
            (self.bytes_transferred as f64 / self.total_bytes as f64) * 100.0
        }
    }
}

/// Represents a file upload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upload {
    /// Unique identifier for the upload
    pub id: String,

    /// Path to the file being uploaded
    pub file_path: PathBuf,

    /// Original filename
    pub filename: String,

    /// Current state of the upload
    pub state: UploadState,

    /// Progress information
    pub progress: UploadProgress,

    /// Creation timestamp
    pub created_at: DateTime<Utc>,

    /// Last modification timestamp
    pub updated_at: DateTime<Utc>,

    /// Upload URL assigned by the server
    pub upload_url: Option<String>,

    /// Additional metadata for the upload
    #[serde(default)]
    pub metadata: HashMap<String, String>,
}

impl Upload {
    pub fn new(file_path: PathBuf, chunk_size: usize) -> TusResult<Self> {
        // 确保文件存在并获取其大小
        let metadata = std::fs::metadata(&file_path)?;

        let filename = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| TusError::Config("Invalid filename".into()))?
            .to_string();

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            file_path,
            filename,
            state: UploadState::Pending,
            progress: UploadProgress::new(metadata.len(), chunk_size),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            upload_url: None,
            metadata: HashMap::new()
        })
    }

    /// 尝试将上传转换到新状态
    pub fn transition_to(&mut self, new_state: UploadState) -> TusResult<()> {
        if !self.state.can_transition_to(new_state) {
            return Err(TusError::InvalidState(
                format!("Cannot transition from {:?} to {:?}", self.state, new_state)
            ));
        }

        self.state = new_state;
        self.updated_at = Utc::now();

        Ok(())
    }

    /// Adds metadata to the upload
    pub fn add_metadata(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.metadata.insert(key.into(), value.into());
        self.updated_at = Utc::now();
    }

    /// Updates the upload progress
    pub fn update_progress(&mut self, bytes: u64, chunk_completed: bool) {
        self.progress.update(bytes, chunk_completed);
        self.updated_at = Utc::now();
    }

    /// Sets the upload URL assigned by the server
    pub fn set_upload_url(&mut self, url: impl Into<String>) {
        self.upload_url = Some(url.into());
        self.updated_at = Utc::now();
    }

    pub fn can_start(&self) -> bool {
        matches!(self.state, UploadState::Pending | UploadState::Paused)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, UploadState::Completed | UploadState::Cancelled | UploadState::Failed)
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, UploadState::Active)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::time::sleep;

    #[test]
    fn test_state_transitions() {
        let transitions = [
            (UploadState::Pending, UploadState::Active, true),
            (UploadState::Active, UploadState::Paused, true),
            (UploadState::Paused, UploadState::Active, true),
            (UploadState::Active, UploadState::Completed, true),
            (UploadState::Completed, UploadState::Active, false),
            (UploadState::Cancelled, UploadState::Active, false),
        ];

        for (from, to, expected) in transitions {
            assert_eq!(from.can_transition_to(to), expected,
                       "Unexpected result for transition {:?} -> {:?}", from, to);
        }
    }

    #[test]
    fn test_progress_tracking() {
        let mut progress = UploadProgress::new(1000, 100);

        // Initial state
        assert_eq!(progress.bytes_transferred, 0);
        assert_eq!(progress.total_bytes, 1000);
        assert_eq!(progress.chunks_completed, 0);
        assert_eq!(progress.total_chunks, 10);

        // Update progress
        progress.update(100, true);
        assert_eq!(progress.bytes_transferred, 100);
        assert_eq!(progress.chunks_completed, 1);
        assert!((progress.percentage() - 10.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_upload_lifecycle() {
        let temp_file = tempfile::NamedTempFile::new().unwrap();
        let path = temp_file.path().to_owned();

        let mut upload = Upload::new(path, 1024).unwrap();

        // Test initial state
        assert_eq!(upload.state, UploadState::Pending);
        assert!(upload.can_start());

        // Test state transitions
        upload.transition_to(UploadState::Active).unwrap();
        assert!(upload.is_active());

        upload.transition_to(UploadState::Paused).unwrap();
        assert!(!upload.is_active());
        assert!(upload.can_start());

        upload.transition_to(UploadState::Active).unwrap();
        upload.transition_to(UploadState::Completed).unwrap();
        assert!(upload.is_finished());

        // Test metadata
        upload.add_metadata("key", "value");
        assert_eq!(upload.metadata.get("key").unwrap(), "value");
    }

    #[tokio::test]
    async fn test_speed_calculation() {
        let mut progress = UploadProgress::new(10000, 1000);

        // Simulate upload speed
        progress.update(1000, true);
        sleep(Duration::from_millis(100)).await;
        progress.update(1000, true);

        assert!(progress.speed > 0.0);
    }
}
