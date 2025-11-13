//! V2 存储适配器
//!
//! 实现 StorageManager 和 S3CompatibleStorage trait，
//! 将 IncrementalStorage 的增量存储 API 适配为标准存储接口

use crate::IncrementalStorage;
use crate::error::StorageError;
use async_trait::async_trait;
use silent_nas_core::{FileMetadata, S3CompatibleStorage, StorageManager};
use std::path::Path;
use std::sync::Arc;

/// V2 存储适配器
///
/// 包装 IncrementalStorage，实现标准的 StorageManager trait
#[derive(Clone)]
pub struct StorageV2Adapter {
    /// 增量存储实例
    storage: Arc<IncrementalStorage>,
}

impl StorageV2Adapter {
    /// 创建新的适配器
    pub fn new(storage: Arc<IncrementalStorage>) -> Self {
        Self { storage }
    }

    /// 获取内部存储引用
    pub fn inner(&self) -> &Arc<IncrementalStorage> {
        &self.storage
    }
}

#[async_trait]
impl StorageManager for StorageV2Adapter {
    type Error = silent_storage_v1::StorageError;

    async fn init(&self) -> std::result::Result<(), Self::Error> {
        self.storage
            .init()
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))
    }

    async fn save_file(
        &self,
        file_id: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        // V2 使用增量存储，这里我们保存第一个版本
        // parent_version_id 为 None 表示创建新文件
        let (_delta, file_version) = self
            .storage
            .save_version(file_id, data, None)
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))?;

        // 转换为 FileMetadata
        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: data.len() as u64,
            hash: file_version.version_id.clone(),
            created_at: file_version.created_at,
            modified_at: file_version.created_at,
        })
    }

    async fn save_at_path(
        &self,
        relative_path: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        // 使用路径作为 file_id
        self.save_file(relative_path, data).await
    }

    async fn read_file(&self, file_id: &str) -> std::result::Result<Vec<u8>, Self::Error> {
        // 读取文件的最新版本
        // 首先获取文件的版本列表
        let versions = self
            .storage
            .list_file_versions(file_id)
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))?;

        if versions.is_empty() {
            return Err(silent_storage_v1::StorageError::Storage(format!(
                "文件不存在: {}",
                file_id
            )));
        }

        // 获取最新版本（list_file_versions 已按时间降序排列）
        let latest_version = &versions[0];

        // 读取版本数据
        self.storage
            .read_version_data(&latest_version.version_id)
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))
    }

    async fn delete_file(&self, file_id: &str) -> std::result::Result<(), Self::Error> {
        // 删除文件及其所有版本
        self.storage
            .delete_file(file_id)
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))
    }

    async fn file_exists(&self, file_id: &str) -> bool {
        // 检查文件是否有版本
        match self.storage.list_file_versions(file_id).await {
            Ok(versions) => !versions.is_empty(),
            Err(_) => false,
        }
    }

    async fn get_metadata(&self, file_id: &str) -> std::result::Result<FileMetadata, Self::Error> {
        let versions = self
            .storage
            .list_file_versions(file_id)
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))?;

        if versions.is_empty() {
            return Err(silent_storage_v1::StorageError::Storage(format!(
                "文件不存在: {}",
                file_id
            )));
        }

        let latest_version = &versions[0];

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_id.to_string(),
            size: latest_version.file_size,
            hash: latest_version.version_id.clone(),
            created_at: latest_version.created_at,
            modified_at: latest_version.created_at,
        })
    }

    async fn list_files(&self) -> std::result::Result<Vec<FileMetadata>, Self::Error> {
        // 从文件索引获取所有文件列表
        let file_ids = self
            .storage
            .list_files()
            .await
            .map_err(|e: StorageError| silent_storage_v1::StorageError::from(e))?;

        let mut files = Vec::new();
        for file_id in file_ids {
            // 获取文件信息
            if let Ok(file_info) = self.storage.get_file_info(&file_id).await {
                // 获取最新版本的详细信息
                if let Ok(version_info) = self
                    .storage
                    .get_version_info(&file_info.latest_version_id)
                    .await
                {
                    files.push(FileMetadata {
                        id: file_id.clone(),
                        name: file_id,
                        path: file_info.latest_version_id.clone(),
                        size: version_info.file_size,
                        hash: version_info.version_id,
                        created_at: file_info.created_at,
                        modified_at: file_info.modified_at,
                    });
                }
            }
        }

        Ok(files)
    }

    async fn verify_hash(
        &self,
        file_id: &str,
        expected_hash: &str,
    ) -> std::result::Result<bool, Self::Error> {
        let metadata = self.get_metadata(file_id).await?;
        Ok(metadata.hash == expected_hash)
    }

    fn root_dir(&self) -> &Path {
        self.storage.version_root()
    }

    fn get_full_path(&self, relative_path: &str) -> std::path::PathBuf {
        self.root_dir().join(relative_path)
    }
}

#[async_trait]
impl S3CompatibleStorage for StorageV2Adapter {
    type Error = silent_storage_v1::StorageError;

    async fn create_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        // V2 中 bucket 可以映射为目录
        let bucket_path = self.root_dir().join(bucket_name);
        tokio::fs::create_dir_all(&bucket_path)
            .await
            .map_err(silent_storage_v1::StorageError::Io)?;
        Ok(())
    }

    async fn delete_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        let bucket_path = self.root_dir().join(bucket_name);
        tokio::fs::remove_dir_all(&bucket_path)
            .await
            .map_err(silent_storage_v1::StorageError::Io)?;
        Ok(())
    }

    async fn bucket_exists(&self, bucket_name: &str) -> bool {
        let bucket_path = self.root_dir().join(bucket_name);
        bucket_path.exists()
    }

    async fn list_buckets(&self) -> std::result::Result<Vec<String>, Self::Error> {
        let mut buckets = Vec::new();
        let mut entries = tokio::fs::read_dir(self.root_dir())
            .await
            .map_err(silent_storage_v1::StorageError::Io)?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(silent_storage_v1::StorageError::Io)?
        {
            if entry
                .file_type()
                .await
                .map_err(silent_storage_v1::StorageError::Io)?
                .is_dir()
            {
                if let Some(name) = entry.file_name().to_str() {
                    buckets.push(name.to_string());
                }
            }
        }

        Ok(buckets)
    }

    async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> std::result::Result<Vec<String>, Self::Error> {
        let bucket_path = self.root_dir().join(bucket_name);
        let mut objects = Vec::new();

        if !bucket_path.exists() {
            return Ok(objects);
        }

        // 递归扫描目录
        fn collect_files(
            dir: &std::path::Path,
            base: &std::path::Path,
            prefix: &str,
            objects: &mut Vec<String>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_file() {
                    if let Ok(relative) = path.strip_prefix(base) {
                        let key = relative.to_string_lossy().to_string();
                        if key.starts_with(prefix) {
                            objects.push(key);
                        }
                    }
                } else if path.is_dir() {
                    collect_files(&path, base, prefix, objects)?;
                }
            }
            Ok(())
        }

        collect_files(&bucket_path, &bucket_path, prefix, &mut objects)
            .map_err(silent_storage_v1::StorageError::Io)?;

        Ok(objects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use silent_storage_v1::StorageManager as StorageV1;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_adapter_basic_operations() {
        let temp_dir = TempDir::new().unwrap();
        let root_path = temp_dir.path().to_str().unwrap();

        // 创建 V1 作为底层存储
        let v1 = Arc::new(StorageV1::new(temp_dir.path().to_path_buf(), 4096));
        v1.init().await.unwrap();

        // 创建 V2 增量存储
        let config = crate::IncrementalConfig::default();
        let v2_storage = Arc::new(IncrementalStorage::new(v1, config, root_path));
        v2_storage.init().await.unwrap();

        // 创建适配器
        let adapter = StorageV2Adapter::new(v2_storage);
        adapter.init().await.unwrap();

        // 测试保存文件
        let data = b"test data";
        let metadata = adapter.save_file("test_file", data).await.unwrap();
        assert_eq!(metadata.id, "test_file");
        assert_eq!(metadata.size, data.len() as u64);

        // 测试读取文件
        let read_data = adapter.read_file("test_file").await.unwrap();
        assert_eq!(read_data, data);

        // 测试文件存在性
        assert!(adapter.file_exists("test_file").await);
        assert!(!adapter.file_exists("non_existent").await);

        // 测试获取元数据
        let meta = adapter.get_metadata("test_file").await.unwrap();
        assert_eq!(meta.id, "test_file");
    }

    #[tokio::test]
    async fn test_adapter_implements_traits() {
        let temp_dir = TempDir::new().unwrap();
        let root_path = temp_dir.path().to_str().unwrap();

        let v1 = Arc::new(StorageV1::new(temp_dir.path().to_path_buf(), 4096));
        let config = crate::IncrementalConfig::default();
        let v2_storage = Arc::new(IncrementalStorage::new(v1, config, root_path));

        let adapter = StorageV2Adapter::new(v2_storage);

        // 验证实现了 StorageManager trait
        let _storage: &dyn StorageManager<Error = StorageError> = &adapter;

        // 验证实现了 S3CompatibleStorage trait
        let _s3: &dyn S3CompatibleStorage<Error = StorageError> = &adapter;
    }
}
