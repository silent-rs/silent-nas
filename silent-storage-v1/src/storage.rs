use crate::error::{Result, StorageError};
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use silent_nas_core::FileMetadata;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{debug, info};

/// 文件存储管理器
#[derive(Clone)]
pub struct StorageManager {
    root_path: PathBuf,
    #[allow(dead_code)]
    chunk_size: usize,
}

impl StorageManager {
    pub fn new(root_path: PathBuf, chunk_size: usize) -> Self {
        Self {
            root_path,
            chunk_size,
        }
    }

    /// 获取根目录路径
    #[allow(dead_code)]
    pub fn root_dir(&self) -> &Path {
        &self.root_path
    }

    /// 初始化存储目录
    pub async fn init(&self) -> Result<()> {
        // 根目录
        fs::create_dir_all(&self.root_path).await?;
        // 数据目录（用于实际文件存储）
        fs::create_dir_all(self.data_root()).await?;
        // 版本目录
        fs::create_dir_all(self.root_path.join("versions")).await?;
        info!(
            "存储目录初始化完成: root={:?}, data={:?}",
            self.root_path,
            self.data_root()
        );
        Ok(())
    }

    /// 数据根目录（root/data）
    fn data_root(&self) -> PathBuf {
        self.root_path.join("data")
    }

    /// 获取文件的完整路径（基于 file_id）
    fn get_file_path(&self, file_id: &str) -> PathBuf {
        self.data_root().join(file_id)
    }

    /// 获取文件的完整路径（基于相对路径，用于 WebDAV）
    #[allow(dead_code)]
    pub fn get_full_path(&self, relative_path: &str) -> PathBuf {
        let path = relative_path.trim_start_matches('/');
        self.data_root().join(path)
    }

    /// 保存文件
    pub async fn save_file(&self, file_id: &str, data: &[u8]) -> Result<FileMetadata> {
        let file_path = self.get_file_path(file_id);

        // 创建父目录
        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 写入文件
        let mut file = fs::File::create(&file_path).await?;
        file.write_all(data).await?;
        file.flush().await?;

        debug!("文件已保存: {:?}", file_path);

        // 计算哈希
        let hash = self.calculate_hash(data);

        // 获取元数据
        let metadata = fs::metadata(&file_path).await?;
        let now = chrono::Local::now().naive_local();

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: file_path.to_string_lossy().to_string(),
            size: metadata.len(),
            hash,
            created_at: now,
            modified_at: now,
        })
    }

    /// 按相对路径保存文件（用于 WebDAV/S3 路径语义）
    pub async fn save_at_path(&self, relative_path: &str, data: &[u8]) -> Result<FileMetadata> {
        let rel = relative_path.trim_start_matches('/');
        let file_path = self.data_root().join(rel);

        if let Some(parent) = file_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let mut file = fs::File::create(&file_path).await?;
        file.write_all(data).await?;
        file.flush().await?;

        debug!("按路径保存文件: {:?}", file_path);

        let hash = self.calculate_hash(data);
        let metadata = fs::metadata(&file_path).await?;
        let now = chrono::Local::now().naive_local();

        Ok(FileMetadata {
            id: rel.to_string(),
            name: file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(rel)
                .to_string(),
            path: format!("/{}", rel),
            size: metadata.len(),
            hash,
            created_at: now,
            modified_at: now,
        })
    }

    /// 读取文件
    pub async fn read_file(&self, file_id: &str) -> Result<Vec<u8>> {
        let file_path = self.get_file_path(file_id);
        // 兼容旧布局：若 data/ 下不存在，则尝试 root/<prefix>/<id>
        let real_path = if file_path.exists() {
            file_path
        } else {
            // 兼容旧布局：data/<prefix>/<id>
            let prefix = &file_id[..2.min(file_id.len())];
            let legacy_data = self.data_root().join(prefix).join(file_id);
            if legacy_data.exists() {
                legacy_data
            } else {
                // 更早的旧布局：root/<prefix>/<id>
                let legacy_root = self.root_path.join(prefix).join(file_id);
                if legacy_root.exists() {
                    legacy_root
                } else {
                    return Err(StorageError::FileNotFound(file_id.to_string()));
                }
            }
        };

        let data = fs::read(&real_path).await?;
        debug!("文件已读取: {:?}, 大小: {} 字节", real_path, data.len());
        Ok(data)
    }

    /// 删除文件
    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        let file_path = self.get_file_path(file_id);
        let real_path = if file_path.exists() {
            file_path
        } else {
            let prefix = &file_id[..2.min(file_id.len())];
            let legacy_data = self.data_root().join(prefix).join(file_id);
            if legacy_data.exists() {
                legacy_data
            } else {
                let legacy_root = self.root_path.join(prefix).join(file_id);
                if legacy_root.exists() {
                    legacy_root
                } else {
                    return Err(StorageError::FileNotFound(file_id.to_string()));
                }
            }
        };

        fs::remove_file(&real_path).await?;
        info!("文件已删除: {:?}", real_path);
        Ok(())
    }

    /// 检查文件是否存在
    #[allow(dead_code)]
    pub async fn file_exists(&self, file_id: &str) -> bool {
        let file_path = self.get_file_path(file_id);
        file_path.exists()
    }

    /// 创建bucket目录
    pub async fn create_bucket(&self, bucket_name: &str) -> Result<()> {
        let bucket_path = self.data_root().join(bucket_name);
        if bucket_path.exists() {
            return Err(StorageError::Storage("Bucket已存在".to_string()));
        }
        fs::create_dir_all(&bucket_path).await?;
        debug!("创建bucket: {}", bucket_name);
        Ok(())
    }

    /// 删除bucket目录
    pub async fn delete_bucket(&self, bucket_name: &str) -> Result<()> {
        let bucket_path = self.data_root().join(bucket_name);
        if !bucket_path.exists() {
            return Err(StorageError::Storage("Bucket不存在".to_string()));
        }

        // 检查bucket是否为空
        let mut entries = fs::read_dir(&bucket_path).await?;
        if entries.next_entry().await?.is_some() {
            return Err(StorageError::Storage("Bucket不为空，无法删除".to_string()));
        }

        fs::remove_dir(&bucket_path).await?;
        debug!("删除bucket: {}", bucket_name);
        Ok(())
    }

    /// 检查bucket是否存在
    pub async fn bucket_exists(&self, bucket_name: &str) -> bool {
        let bucket_path = self.data_root().join(bucket_name);
        let exists = bucket_path.exists();
        let is_dir = if exists { bucket_path.is_dir() } else { false };
        debug!(
            "bucket_exists: bucket={}, path={:?}, exists={}, is_dir={}",
            bucket_name, bucket_path, exists, is_dir
        );
        exists && is_dir
    }

    /// 列出所有buckets
    pub async fn list_buckets(&self) -> Result<Vec<String>> {
        let mut buckets = Vec::new();
        let mut entries = fs::read_dir(self.data_root()).await?;

        while let Some(entry) = entries.next_entry().await? {
            if let Ok(metadata) = entry.metadata().await
                && metadata.is_dir()
                && let Some(name) = entry.file_name().to_str()
            {
                buckets.push(name.to_string());
            }
        }

        buckets.sort();
        Ok(buckets)
    }

    /// 列出bucket中的所有对象
    pub async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> Result<Vec<String>> {
        let bucket_path = self.data_root().join(bucket_name);
        if !bucket_path.exists() {
            return Ok(Vec::new());
        }

        let mut objects = Vec::new();
        self.scan_bucket_directory(&bucket_path, &bucket_path, prefix, &mut objects)
            .await?;
        objects.sort();
        Ok(objects)
    }

    /// 递归扫描bucket目录
    #[allow(clippy::only_used_in_recursion)]
    fn scan_bucket_directory<'a>(
        &'a self,
        current_path: &'a Path,
        base_path: &'a Path,
        prefix: &'a str,
        objects: &'a mut Vec<String>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = fs::read_dir(current_path).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if let Ok(metadata) = entry.metadata().await {
                    if metadata.is_file() {
                        // 获取相对路径
                        if let Ok(relative_path) = path.strip_prefix(base_path)
                            && let Some(key) = relative_path.to_str()
                        {
                            let key = key.replace('\\', "/");
                            if prefix.is_empty() || key.starts_with(prefix) {
                                objects.push(key);
                            }
                        }
                    } else if metadata.is_dir() {
                        // 递归扫描子目录
                        self.scan_bucket_directory(&path, base_path, prefix, objects)
                            .await?;
                    }
                }
            }
            Ok(())
        })
    }

    /// 获取文件元数据
    pub async fn get_metadata(&self, file_id: &str) -> Result<FileMetadata> {
        let file_path = self.get_file_path(file_id);
        // 兼容旧布局
        let real_path = if file_path.exists() {
            file_path
        } else {
            let prefix = &file_id[..2.min(file_id.len())];
            let legacy_data = self.data_root().join(prefix).join(file_id);
            if legacy_data.exists() {
                legacy_data
            } else {
                let legacy_root = self.root_path.join(prefix).join(file_id);
                if legacy_root.exists() {
                    legacy_root
                } else {
                    return Err(StorageError::FileNotFound(file_id.to_string()));
                }
            }
        };

        let metadata = fs::metadata(&real_path).await?;
        let data = fs::read(&real_path).await?;
        let hash = self.calculate_hash(&data);

        let now = chrono::Local::now().naive_local();

        Ok(FileMetadata {
            id: file_id.to_string(),
            name: file_id.to_string(),
            path: real_path.to_string_lossy().to_string(),
            size: metadata.len(),
            hash,
            created_at: now,
            modified_at: now,
        })
    }

    /// 列出所有文件
    pub async fn list_files(&self) -> Result<Vec<FileMetadata>> {
        let mut files = Vec::new();
        self.scan_directory(&self.data_root(), &mut files).await?;
        Ok(files)
    }

    /// 递归扫描目录
    fn scan_directory<'a>(
        &'a self,
        dir: &'a Path,
        files: &'a mut Vec<FileMetadata>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            let mut entries = fs::read_dir(dir).await?;

            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();

                if path.is_dir() {
                    self.scan_directory(&path, files).await?;
                } else if path.is_file()
                    && let Some(file_name) = path.file_name().and_then(|n| n.to_str())
                    && let Ok(metadata) = self.get_metadata(file_name).await
                {
                    files.push(metadata);
                }
            }

            Ok(())
        })
    }

    /// 计算文件哈希值 (SHA-256)
    fn calculate_hash(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// 验证文件哈希
    #[allow(dead_code)]
    pub async fn verify_hash(&self, file_id: &str, expected_hash: &str) -> Result<bool> {
        let data = self.read_file(file_id).await?;
        let actual_hash = self.calculate_hash(&data);
        Ok(actual_hash == expected_hash)
    }
}

/// 为 StorageManager 实现 silent_nas_core::StorageManagerTrait trait
#[async_trait]
impl silent_nas_core::StorageManagerTrait for StorageManager {
    type Error = StorageError;

    async fn init(&self) -> std::result::Result<(), Self::Error> {
        StorageManager::init(self).await
    }

    async fn save_file(
        &self,
        file_id: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        StorageManager::save_file(self, file_id, data).await
    }

    async fn save_at_path(
        &self,
        relative_path: &str,
        data: &[u8],
    ) -> std::result::Result<FileMetadata, Self::Error> {
        StorageManager::save_at_path(self, relative_path, data).await
    }

    async fn read_file(&self, file_id: &str) -> std::result::Result<Vec<u8>, Self::Error> {
        StorageManager::read_file(self, file_id).await
    }

    async fn delete_file(&self, file_id: &str) -> std::result::Result<(), Self::Error> {
        StorageManager::delete_file(self, file_id).await
    }

    async fn file_exists(&self, file_id: &str) -> bool {
        StorageManager::file_exists(self, file_id).await
    }

    async fn get_metadata(&self, file_id: &str) -> std::result::Result<FileMetadata, Self::Error> {
        StorageManager::get_metadata(self, file_id).await
    }

    async fn list_files(&self) -> std::result::Result<Vec<FileMetadata>, Self::Error> {
        StorageManager::list_files(self).await
    }

    async fn verify_hash(
        &self,
        file_id: &str,
        expected_hash: &str,
    ) -> std::result::Result<bool, Self::Error> {
        StorageManager::verify_hash(self, file_id, expected_hash).await
    }

    fn root_dir(&self) -> &Path {
        &self.root_path
    }

    fn get_full_path(&self, relative_path: &str) -> PathBuf {
        StorageManager::get_full_path(self, relative_path)
    }
}

/// 为 StorageManager 实现 S3CompatibleStorageTrait trait
#[async_trait]
impl silent_nas_core::S3CompatibleStorageTrait for StorageManager {
    type Error = StorageError;

    async fn create_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        StorageManager::create_bucket(self, bucket_name).await
    }

    async fn delete_bucket(&self, bucket_name: &str) -> std::result::Result<(), Self::Error> {
        StorageManager::delete_bucket(self, bucket_name).await
    }

    async fn bucket_exists(&self, bucket_name: &str) -> bool {
        StorageManager::bucket_exists(self, bucket_name).await
    }

    async fn list_buckets(&self) -> std::result::Result<Vec<String>, Self::Error> {
        StorageManager::list_buckets(self).await
    }

    async fn list_bucket_objects(
        &self,
        bucket_name: &str,
        prefix: &str,
    ) -> std::result::Result<Vec<String>, Self::Error> {
        StorageManager::list_bucket_objects(self, bucket_name, prefix).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_storage() -> (StorageManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(temp_dir.path().to_path_buf(), 4 * 1024 * 1024);
        (storage, temp_dir)
    }

    #[tokio::test]
    async fn test_storage_new() {
        let (storage, _temp) = create_test_storage();
        assert_eq!(storage.chunk_size, 4 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_storage_init() {
        let (storage, _temp) = create_test_storage();
        let result = storage.init().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_save_and_read_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = b"Hello, World!";
        let metadata = storage.save_file("test_file", data).await.unwrap();

        assert_eq!(metadata.size, data.len() as u64);
        assert_eq!(metadata.id, "test_file");
        assert!(!metadata.hash.is_empty());

        let read_data = storage.read_file("test_file").await.unwrap();
        assert_eq!(read_data, data);
    }

    // 其他测试省略，与原测试相同...
}
