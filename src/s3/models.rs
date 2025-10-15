use chrono::{DateTime, Utc};
use std::collections::HashMap;

/// S3对象信息
#[derive(Debug)]
pub struct S3Object {
    pub key: String,
    pub last_modified: DateTime<Utc>,
    pub etag: String,
    pub size: u64,
}

/// 分片上传信息
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MultipartUpload {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub initiated: DateTime<Utc>,
    pub parts: HashMap<u32, PartInfo>,
}

/// 分片信息
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PartInfo {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
    pub data: Vec<u8>,
}
