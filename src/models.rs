use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};

/// 文件元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMetadata {
    /// 文件 ID (scru128)
    pub id: String,
    /// 文件名
    pub name: String,
    /// 相对路径
    pub path: String,
    /// 文件大小（字节）
    pub size: u64,
    /// SHA-256 哈希值
    pub hash: String,
    /// 创建时间（本地时间）
    pub created_at: NaiveDateTime,
    /// 修改时间（本地时间）
    pub modified_at: NaiveDateTime,
}

/// 文件事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Created,
    Modified,
    Deleted,
}

/// 文件变更事件
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEvent {
    /// 事件 ID (scru128)
    pub event_id: String,
    /// 事件类型
    pub event_type: EventType,
    /// 文件 ID
    pub file_id: String,
    /// 时间戳（本地时间）
    pub timestamp: NaiveDateTime,
    /// 文件元数据
    pub metadata: Option<FileMetadata>,
}

impl FileEvent {
    pub fn new(event_type: EventType, file_id: String, metadata: Option<FileMetadata>) -> Self {
        Self {
            event_id: scru128::new_string(),
            event_type,
            file_id,
            timestamp: chrono::Local::now().naive_local(),
            metadata,
        }
    }
}

/// 文件版本元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct FileVersion {
    /// 版本 ID (scru128)
    pub version_id: String,
    /// 文件 ID
    pub file_id: String,
    /// 文件名（版本创建时的文件名）
    pub name: String,
    /// 文件大小（字节）
    pub size: u64,
    /// SHA-256 哈希值
    pub hash: String,
    /// 创建时间（本地时间）
    pub created_at: NaiveDateTime,
    /// 创建者（可选）
    pub author: Option<String>,
    /// 版本说明（可选）
    pub comment: Option<String>,
    /// 是否为当前版本
    pub is_current: bool,
}

impl FileVersion {
    #[allow(dead_code)]
    pub fn new(
        file_id: String,
        name: String,
        size: u64,
        hash: String,
        author: Option<String>,
        comment: Option<String>,
    ) -> Self {
        Self {
            version_id: scru128::new_string(),
            file_id,
            name,
            size,
            hash,
            created_at: chrono::Local::now().naive_local(),
            author,
            comment,
            is_current: false,
        }
    }

    /// 从文件元数据创建版本
    pub fn from_metadata(metadata: &FileMetadata, author: Option<String>) -> Self {
        Self {
            version_id: scru128::new_string(),
            file_id: metadata.id.clone(),
            name: metadata.name.clone(),
            size: metadata.size,
            hash: metadata.hash.clone(),
            created_at: chrono::Local::now().naive_local(),
            author,
            comment: None,
            is_current: true,
        }
    }
}
