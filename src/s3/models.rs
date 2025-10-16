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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_object_creation() {
        let obj = S3Object {
            key: "test/file.txt".to_string(),
            last_modified: Utc::now(),
            etag: "abc123".to_string(),
            size: 1024,
        };

        assert_eq!(obj.key, "test/file.txt");
        assert_eq!(obj.etag, "abc123");
        assert_eq!(obj.size, 1024);
    }

    #[test]
    fn test_s3_object_debug() {
        let obj = S3Object {
            key: "test.txt".to_string(),
            last_modified: Utc::now(),
            etag: "etag123".to_string(),
            size: 512,
        };

        let debug_str = format!("{:?}", obj);
        assert!(debug_str.contains("test.txt"));
        assert!(debug_str.contains("etag123"));
    }

    #[test]
    fn test_part_info_creation() {
        let part = PartInfo {
            part_number: 1,
            etag: "part_etag".to_string(),
            size: 5242880,
            data: vec![1, 2, 3, 4, 5],
        };

        assert_eq!(part.part_number, 1);
        assert_eq!(part.etag, "part_etag");
        assert_eq!(part.size, 5242880);
        assert_eq!(part.data.len(), 5);
    }

    #[test]
    fn test_part_info_clone() {
        let part = PartInfo {
            part_number: 2,
            etag: "etag2".to_string(),
            size: 1024,
            data: vec![10, 20, 30],
        };

        let cloned = part.clone();
        assert_eq!(cloned.part_number, part.part_number);
        assert_eq!(cloned.etag, part.etag);
        assert_eq!(cloned.size, part.size);
        assert_eq!(cloned.data, part.data);
    }

    #[test]
    fn test_multipart_upload_creation() {
        let mut parts = HashMap::new();
        parts.insert(
            1,
            PartInfo {
                part_number: 1,
                etag: "part1".to_string(),
                size: 1024,
                data: vec![],
            },
        );

        let upload = MultipartUpload {
            upload_id: "upload123".to_string(),
            bucket: "my-bucket".to_string(),
            key: "my-key".to_string(),
            initiated: Utc::now(),
            parts,
        };

        assert_eq!(upload.upload_id, "upload123");
        assert_eq!(upload.bucket, "my-bucket");
        assert_eq!(upload.key, "my-key");
        assert_eq!(upload.parts.len(), 1);
    }

    #[test]
    fn test_multipart_upload_clone() {
        let upload = MultipartUpload {
            upload_id: "id1".to_string(),
            bucket: "bucket1".to_string(),
            key: "key1".to_string(),
            initiated: Utc::now(),
            parts: HashMap::new(),
        };

        let cloned = upload.clone();
        assert_eq!(cloned.upload_id, upload.upload_id);
        assert_eq!(cloned.bucket, upload.bucket);
        assert_eq!(cloned.key, upload.key);
    }

    #[test]
    fn test_multipart_upload_with_multiple_parts() {
        let mut parts = HashMap::new();
        for i in 1..=5 {
            parts.insert(
                i,
                PartInfo {
                    part_number: i,
                    etag: format!("etag{}", i),
                    size: 1024 * i as u64,
                    data: vec![i as u8; 10],
                },
            );
        }

        let upload = MultipartUpload {
            upload_id: "multi_upload".to_string(),
            bucket: "test-bucket".to_string(),
            key: "large-file.bin".to_string(),
            initiated: Utc::now(),
            parts,
        };

        assert_eq!(upload.parts.len(), 5);
        assert!(upload.parts.contains_key(&1));
        assert!(upload.parts.contains_key(&5));
        assert_eq!(upload.parts.get(&3).unwrap().etag, "etag3");
    }

    #[test]
    fn test_part_info_empty_data() {
        let part = PartInfo {
            part_number: 1,
            etag: "empty".to_string(),
            size: 0,
            data: vec![],
        };

        assert_eq!(part.data.len(), 0);
        assert_eq!(part.size, 0);
    }

    #[test]
    fn test_part_info_large_data() {
        let large_data = vec![0u8; 1024 * 1024]; // 1MB
        let part = PartInfo {
            part_number: 1,
            etag: "large".to_string(),
            size: 1024 * 1024,
            data: large_data,
        };

        assert_eq!(part.data.len(), 1024 * 1024);
        assert_eq!(part.size, 1024 * 1024);
    }

    #[test]
    fn test_s3_object_with_path() {
        let obj = S3Object {
            key: "folder1/folder2/file.txt".to_string(),
            last_modified: Utc::now(),
            etag: "hash".to_string(),
            size: 2048,
        };

        assert!(obj.key.contains('/'));
        assert_eq!(obj.key.matches('/').count(), 2);
    }
}
