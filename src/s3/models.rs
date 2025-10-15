use chrono::{DateTime, Utc};

/// S3对象信息
#[derive(Debug)]
pub struct S3Object {
    pub key: String,
    pub last_modified: DateTime<Utc>,
    pub etag: String,
    pub size: u64,
}
