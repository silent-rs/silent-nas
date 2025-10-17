use crate::error::{NasError, Result};
use crate::models::FileMetadata;
use sha2::{Digest, Sha256};
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
        // 所有ID统一直接映射到 data/<id>，不再使用哈希前缀目录
        // 若file_id包含路径分隔符（如S3 bucket/key），保持相对路径写入 data/<bucket>/<key>
        // 若包含分隔符，视为相对路径；否则视为ID，均直接放在 data/ 下（路径保持相对结构）
        self.data_root().join(file_id)
    }

    /// 获取文件的完整路径（基于相对路径，用于 WebDAV）
    #[allow(dead_code)]
    pub fn get_full_path(&self, relative_path: &str) -> PathBuf {
        let path = relative_path.trim_start_matches('/');
        // 将对外路径映射到 data/ 下
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
                    return Err(NasError::FileNotFound(file_id.to_string()));
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
                    return Err(NasError::FileNotFound(file_id.to_string()));
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
            return Err(NasError::Storage("Bucket已存在".to_string()));
        }
        fs::create_dir_all(&bucket_path).await?;
        debug!("创建bucket: {}", bucket_name);
        Ok(())
    }

    /// 删除bucket目录
    pub async fn delete_bucket(&self, bucket_name: &str) -> Result<()> {
        let bucket_path = self.data_root().join(bucket_name);
        if !bucket_path.exists() {
            return Err(NasError::Storage("Bucket不存在".to_string()));
        }

        // 检查bucket是否为空
        let mut entries = fs::read_dir(&bucket_path).await?;
        if entries.next_entry().await?.is_some() {
            return Err(NasError::Storage("Bucket不为空，无法删除".to_string()));
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
                    return Err(NasError::FileNotFound(file_id.to_string()));
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

    #[tokio::test]
    async fn test_save_empty_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = b"";
        let metadata = storage.save_file("empty_file", data).await.unwrap();
        assert_eq!(metadata.size, 0);
    }

    #[tokio::test]
    async fn test_save_large_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = vec![0u8; 1024 * 1024]; // 1MB
        let metadata = storage.save_file("large_file", &data).await.unwrap();
        assert_eq!(metadata.size, 1024 * 1024);
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let result = storage.read_file("nonexistent").await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, NasError::FileNotFound(_)));
        }
    }

    #[tokio::test]
    async fn test_delete_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = b"test data";
        storage.save_file("delete_test", data).await.unwrap();

        let result = storage.delete_file("delete_test").await;
        assert!(result.is_ok());

        let read_result = storage.read_file("delete_test").await;
        assert!(read_result.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_file() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let result = storage.delete_file("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_exists() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        assert!(!storage.file_exists("test").await);

        storage.save_file("test", b"data").await.unwrap();
        assert!(storage.file_exists("test").await);
    }

    #[tokio::test]
    async fn test_get_metadata() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = b"metadata test";
        storage.save_file("meta_test", data).await.unwrap();

        let metadata = storage.get_metadata("meta_test").await.unwrap();
        assert_eq!(metadata.id, "meta_test");
        assert_eq!(metadata.size, data.len() as u64);
    }

    #[tokio::test]
    async fn test_get_metadata_nonexistent() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let result = storage.get_metadata("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_calculate_hash() {
        let (storage, _temp) = create_test_storage();

        let data1 = b"test data";
        let hash1 = storage.calculate_hash(data1);
        assert!(!hash1.is_empty());
        assert_eq!(hash1.len(), 64); // SHA-256 = 64 hex chars

        let data2 = b"test data";
        let hash2 = storage.calculate_hash(data2);
        assert_eq!(hash1, hash2); // Same data = same hash

        let data3 = b"different data";
        let hash3 = storage.calculate_hash(data3);
        assert_ne!(hash1, hash3); // Different data = different hash
    }

    #[tokio::test]
    async fn test_list_files() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let files = storage.list_files().await.unwrap();
        assert_eq!(files.len(), 0);

        storage.save_file("file1", b"data1").await.unwrap();
        storage.save_file("file2", b"data2").await.unwrap();

        let files = storage.list_files().await.unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn test_get_file_path() {
        let (storage, _temp) = create_test_storage();

        let path1 = storage.get_file_path("abc123");
        assert!(path1.to_string_lossy().contains("ab"));
        assert!(path1.to_string_lossy().contains("abc123"));

        let path2 = storage.get_file_path("bucket/key");
        assert!(path2.to_string_lossy().contains("bucket/key"));
    }

    #[tokio::test]
    async fn test_get_full_path() {
        let (storage, _temp) = create_test_storage();

        let path1 = storage.get_full_path("/test/file.txt");
        assert!(path1.to_string_lossy().contains("test/file.txt"));

        let path2 = storage.get_full_path("test/file.txt");
        assert!(path2.to_string_lossy().contains("test/file.txt"));
    }

    #[tokio::test]
    async fn test_storage_clone() {
        let (storage, _temp) = create_test_storage();
        let cloned = storage.clone();

        assert_eq!(storage.root_path, cloned.root_path);
        assert_eq!(storage.chunk_size, cloned.chunk_size);
    }

    #[tokio::test]
    async fn test_multiple_operations() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        // 保存多个文件
        for i in 0..5 {
            let file_id = format!("file_{}", i);
            let data = format!("data_{}", i).into_bytes();
            storage.save_file(&file_id, &data).await.unwrap();
        }

        // 验证所有文件
        for i in 0..5 {
            let file_id = format!("file_{}", i);
            let data = storage.read_file(&file_id).await.unwrap();
            assert_eq!(data, format!("data_{}", i).into_bytes());
        }

        // 删除部分文件
        for i in 0..3 {
            let file_id = format!("file_{}", i);
            storage.delete_file(&file_id).await.unwrap();
        }

        // 验证删除结果
        for i in 0..3 {
            let file_id = format!("file_{}", i);
            assert!(storage.read_file(&file_id).await.is_err());
        }
        for i in 3..5 {
            let file_id = format!("file_{}", i);
            assert!(storage.read_file(&file_id).await.is_ok());
        }
    }

    #[tokio::test]
    async fn test_create_bucket() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let result = storage.create_bucket("test-bucket").await;
        assert!(result.is_ok());

        assert!(storage.bucket_exists("test-bucket").await);
    }

    #[tokio::test]
    async fn test_create_bucket_duplicate() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();
        let result = storage.create_bucket("test-bucket").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_bucket() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();
        assert!(storage.bucket_exists("test-bucket").await);

        let result = storage.delete_bucket("test-bucket").await;
        assert!(result.is_ok());
        assert!(!storage.bucket_exists("test-bucket").await);
    }

    #[tokio::test]
    async fn test_delete_nonexistent_bucket() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let result = storage.delete_bucket("nonexistent").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_nonempty_bucket() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();

        // 在bucket中添加文件
        let file_path = "test-bucket/test.txt";
        storage.save_file(file_path, b"data").await.unwrap();

        let result = storage.delete_bucket("test-bucket").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_bucket_exists() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        assert!(!storage.bucket_exists("test-bucket").await);

        storage.create_bucket("test-bucket").await.unwrap();
        assert!(storage.bucket_exists("test-bucket").await);
    }

    #[tokio::test]
    async fn test_list_buckets() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let buckets = storage.list_buckets().await.unwrap();
        assert_eq!(buckets.len(), 0);

        storage.create_bucket("bucket1").await.unwrap();
        storage.create_bucket("bucket2").await.unwrap();
        storage.create_bucket("bucket3").await.unwrap();

        let buckets = storage.list_buckets().await.unwrap();
        assert_eq!(buckets.len(), 3);
        assert!(buckets.contains(&"bucket1".to_string()));
        assert!(buckets.contains(&"bucket2".to_string()));
        assert!(buckets.contains(&"bucket3".to_string()));

        // 应该按字母顺序排序
        assert_eq!(buckets[0], "bucket1");
        assert_eq!(buckets[1], "bucket2");
        assert_eq!(buckets[2], "bucket3");
    }

    #[tokio::test]
    async fn test_list_bucket_objects_empty() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();

        let objects = storage
            .list_bucket_objects("test-bucket", "")
            .await
            .unwrap();
        assert_eq!(objects.len(), 0);
    }

    #[tokio::test]
    async fn test_list_bucket_objects() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();

        // 添加文件
        storage
            .save_file("test-bucket/file1.txt", b"data1")
            .await
            .unwrap();
        storage
            .save_file("test-bucket/file2.txt", b"data2")
            .await
            .unwrap();
        storage
            .save_file("test-bucket/dir/file3.txt", b"data3")
            .await
            .unwrap();

        let objects = storage
            .list_bucket_objects("test-bucket", "")
            .await
            .unwrap();
        assert_eq!(objects.len(), 3);
    }

    #[tokio::test]
    async fn test_list_bucket_objects_with_prefix() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.create_bucket("test-bucket").await.unwrap();

        storage
            .save_file("test-bucket/docs/file1.txt", b"data1")
            .await
            .unwrap();
        storage
            .save_file("test-bucket/docs/file2.txt", b"data2")
            .await
            .unwrap();
        storage
            .save_file("test-bucket/images/file3.png", b"data3")
            .await
            .unwrap();

        let objects = storage
            .list_bucket_objects("test-bucket", "docs")
            .await
            .unwrap();
        assert_eq!(objects.len(), 2);
        assert!(objects.iter().all(|o| o.starts_with("docs")));
    }

    #[tokio::test]
    async fn test_verify_hash_correct() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        let data = b"test data for hash";
        let metadata = storage.save_file("test_file", data).await.unwrap();

        let valid = storage
            .verify_hash("test_file", &metadata.hash)
            .await
            .unwrap();
        assert!(valid);
    }

    #[tokio::test]
    async fn test_verify_hash_incorrect() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        storage.save_file("test_file", b"test data").await.unwrap();

        let valid = storage
            .verify_hash("test_file", "wrong_hash")
            .await
            .unwrap();
        assert!(!valid);
    }

    #[tokio::test]
    async fn test_bucket_with_special_chars() {
        let (storage, _temp) = create_test_storage();
        storage.init().await.unwrap();

        // 测试带特殊字符的bucket名称
        let bucket_names = vec!["test-bucket", "test_bucket", "test.bucket"];

        for name in bucket_names {
            let result = storage.create_bucket(name).await;
            assert!(result.is_ok());
            assert!(storage.bucket_exists(name).await);
        }
    }
}
