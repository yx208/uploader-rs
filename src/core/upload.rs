use std::collections::HashMap;
use std::path::PathBuf;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::core::error::{UploadError, UploadResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UploadProgress {
    /// 已传输的字节数
    pub bytes_transferred: u64,

    /// 总字节数
    pub total_bytes: u64,

    /// 当前传输速度
    pub speed: u64,

    /// 最后更新时间
    pub last_update: DateTime<Utc>,
}

impl UploadProgress {
    /// 创建实例
    pub fn new(total_bytes: u64) -> Self {
        Self {
            total_bytes,
            bytes_transferred: 0,
            speed: 0,
            last_update: Utc::now(),
        }
    }

    /// 更新
    pub fn update(&mut self, new_bytes: u64) {
        let now = Utc::now();
        let duration = (now - self.last_update).num_milliseconds() as u64 / 1000;

        if duration > 0 {
            self.speed = new_bytes / duration;
        }

        self.bytes_transferred += new_bytes;
        self.last_update = now;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upload {
    /// 上传文件的唯一 id
    pub id: String,

    /// 上传文件的本地路径
    pub file_path: PathBuf,

    /// 上传文件的名称
    pub filename: String,

    /// 上传状态
    pub status: UploadStatus,

    /// 上传总字节数
    pub total_bytes: u64,

    /// Tus 创建的资源路径
    pub location: Option<String>,

    /// 每次上传的块大小
    pub chunk_size: usize,

    /// 进度
    pub progress: UploadProgress,

    /// 元数据
    #[serde(default)]
    pub metadata: HashMap<String, String>,

    /// 创建时间
    pub created_at: DateTime<Utc>,

    /// 更新时间
    pub update_at: DateTime<Utc>,
}

impl Upload {
    pub fn new(file_path: PathBuf, chunk_size: usize) -> UploadResult<Self> {
        let metadata = std::fs::metadata(file_path.clone())?;
        let filename = file_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| UploadError::ConfigError("Invalid file name".to_string()))?
            .to_string();

        Ok(Self {
            id: Uuid::new_v4().to_string(),
            file_path,
            filename,
            chunk_size,
            location: None,
            total_bytes: metadata.len(),
            status: UploadStatus::Pending,
            progress: UploadProgress::new(metadata.len()),
            created_at: Utc::now(),
            update_at: Utc::now(),
            metadata: HashMap::new()
        })
    }

    pub fn transition_to(&mut self, status: UploadStatus) -> UploadResult<()> {
        if !self.status.can_transition_to(status) {
            return Err(UploadError::InvalidState(
                format!("Cannot transition from {:?} to {:?}", self.status, status)
            ));
        }

        self.status = status;
        self.update_at = Utc::now();

        Ok(())
    }

    pub fn set_location(&mut self, location: impl Into<String>) {
        self.location = Some(location.into());
        self.update_at = Utc::now();
    }

    pub fn is_active(&self) -> bool {
        matches!(self.status, UploadStatus::Active)
    }

    pub fn can_start(&self) -> bool {
        matches!(self.status, UploadStatus::Pending | UploadStatus::Paused)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.status, UploadStatus::Completed | UploadStatus::Failed)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
pub enum UploadStatus {
    /// 已创建，但尚未开始
    Pending,

    /// 正在传输
    Active,

    /// 上传暂时停止，但可以恢复
    Paused,

    /// 上传已成功完成
    Completed,

    /// 上传遇到错误
    Failed,
}

impl UploadStatus {
    pub fn can_transition_to(&self, target: UploadStatus) -> bool {
        use UploadStatus::*;
        match (*self, target) {
            (Pending, Active) => true,

            (Active, Paused) => true,
            (Active, Completed) => true,
            (Active, Failed) => true,

            (Paused, Pending) => true,
            (Paused, Active) => true,

            (Failed, Pending) => true,
            (Failed, Active) => true,

            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_transitions() {
        let transitions = [
            (UploadStatus::Pending, UploadStatus::Active, true),
            (UploadStatus::Active, UploadStatus::Paused, true),
            (UploadStatus::Paused, UploadStatus::Active, true),
            (UploadStatus::Active, UploadStatus::Completed, true),
            (UploadStatus::Completed, UploadStatus::Active, false),
            (UploadStatus::Failed, UploadStatus::Completed, false),
        ];

        for (from, to, expected) in transitions {
            assert_eq!(from.can_transition_to(to), expected,
                       "Unexpected result for transition {:?} -> {:?}", from, to);
        }
    }

    #[test]
    fn test_progress_update() {
        let total_bytes = 1024 * 1024 * 10; // 10MB
        let mut progress = UploadProgress::new(total_bytes);
        assert_eq!(progress.bytes_transferred, 0);

        progress.update(1024 * 1024 * 2);
        assert_eq!(progress.bytes_transferred, 1024 * 1024 * 2);

        progress.update(1024 * 1024 * 2);
        assert_eq!(progress.bytes_transferred, 1024 * 1024 * 4);

        progress.update(1024 * 1024 * 4);
        assert_eq!(progress.bytes_transferred, 1024 * 1024 * 8);
    }
}
