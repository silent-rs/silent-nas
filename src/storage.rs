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

    /// 初始化存储目录
    pub async fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.root_path).await?;
        info!("存储目录初始化完成: {:?}", self.root_path);
        Ok(())
    }

    /// 获取文件的完整路径（基于 file_id）
    fn get_file_path(&self, file_id: &str) -> PathBuf {
        // 如果file_id包含斜杠，说明是bucket/key格式，直接使用
        if file_id.contains('/') {
            self.root_path.join(file_id)
        } else {
            // 使用前2个字符作为子目录，避免单目录文件过多
            let prefix = &file_id[..2.min(file_id.len())];
            self.root_path.join(prefix).join(file_id)
        }
    }

    /// 获取文件的完整路径（基于相对路径，用于 WebDAV）
    #[allow(dead_code)]
    pub fn get_full_path(&self, relative_path: &str) -> PathBuf {
        let path = relative_path.trim_start_matches('/');
        self.root_path.join(path)
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

        if !file_path.exists() {
            return Err(NasError::FileNotFound(file_id.to_string()));
        }

        let data = fs::read(&file_path).await?;
        debug!("文件已读取: {:?}, 大小: {} 字节", file_path, data.len());
        Ok(data)
    }

    /// 删除文件
    pub async fn delete_file(&self, file_id: &str) -> Result<()> {
        let file_path = self.get_file_path(file_id);

        if !file_path.exists() {
            return Err(NasError::FileNotFound(file_id.to_string()));
        }

        fs::remove_file(&file_path).await?;
        info!("文件已删除: {:?}", file_path);
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
        let bucket_path = self.root_path.join(bucket_name);
        if bucket_path.exists() {
            return Err(NasError::Storage("Bucket已存在".to_string()));
        }
        fs::create_dir_all(&bucket_path).await?;
        debug!("创建bucket: {}", bucket_name);
        Ok(())
    }

    /// 删除bucket目录
    pub async fn delete_bucket(&self, bucket_name: &str) -> Result<()> {
        let bucket_path = self.root_path.join(bucket_name);
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
        let bucket_path = self.root_path.join(bucket_name);
        bucket_path.exists() && bucket_path.is_dir()
    }

    /// 列出所有buckets
    pub async fn list_buckets(&self) -> Result<Vec<String>> {
        let mut buckets = Vec::new();
        let mut entries = fs::read_dir(&self.root_path).await?;

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
        let bucket_path = self.root_path.join(bucket_name);
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

        if !file_path.exists() {
            return Err(NasError::FileNotFound(file_id.to_string()));
        }

        let metadata = fs::metadata(&file_path).await?;
        let data = fs::read(&file_path).await?;
        let hash = self.calculate_hash(&data);

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

    /// 列出所有文件
    pub async fn list_files(&self) -> Result<Vec<FileMetadata>> {
        let mut files = Vec::new();
        self.scan_directory(&self.root_path, &mut files).await?;
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

    #[tokio::test]
    async fn test_storage_basic_operations() {
        let temp_dir = std::env::temp_dir().join("silent-nas-test");
        let storage = StorageManager::new(temp_dir.clone(), 4096);

        storage.init().await.unwrap();

        let file_id = scru128::new_string();
        let data = b"Hello, Silent-NAS!";

        // 保存文件
        let metadata = storage.save_file(&file_id, data).await.unwrap();
        assert_eq!(metadata.size, data.len() as u64);

        // 读取文件
        let read_data = storage.read_file(&file_id).await.unwrap();
        assert_eq!(read_data, data);

        // 验证哈希
        let valid = storage.verify_hash(&file_id, &metadata.hash).await.unwrap();
        assert!(valid);

        // 删除文件
        storage.delete_file(&file_id).await.unwrap();
        assert!(!storage.file_exists(&file_id).await);

        // 清理
        let _ = fs::remove_dir_all(temp_dir).await;
    }
}
