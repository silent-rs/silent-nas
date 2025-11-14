//! 可靠性增强模块
//!
//! 提供 WAL、数据校验、自动修复和孤儿资源清理功能

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

/// WAL 操作类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WalOperation {
    /// 创建文件版本
    CreateVersion {
        file_id: String,
        version_id: String,
        chunk_hashes: Vec<String>,
    },
    /// 删除文件版本
    DeleteVersion {
        file_id: String,
        version_id: String,
    },
    /// 删除文件
    DeleteFile { file_id: String },
    /// 垃圾回收
    GarbageCollect { chunk_hashes: Vec<String> },
}

/// WAL 日志条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalEntry {
    /// 序列号
    pub sequence: u64,
    /// 时间戳
    pub timestamp: chrono::NaiveDateTime,
    /// 操作类型
    pub operation: WalOperation,
    /// 校验和
    pub checksum: String,
}

impl WalEntry {
    /// 创建新的 WAL 条目
    pub fn new(sequence: u64, operation: WalOperation) -> Self {
        let timestamp = chrono::Local::now().naive_local();
        let mut entry = Self {
            sequence,
            timestamp,
            operation,
            checksum: String::new(),
        };
        entry.checksum = entry.calculate_checksum();
        entry
    }

    /// 计算校验和
    fn calculate_checksum(&self) -> String {
        let data = format!(
            "{}|{}|{}",
            self.sequence,
            self.timestamp,
            serde_json::to_string(&self.operation).unwrap_or_default()
        );
        let mut hasher = Sha256::new();
        hasher.update(data.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// 验证校验和
    pub fn verify_checksum(&self) -> bool {
        let expected = self.calculate_checksum();
        self.checksum == expected
    }
}

/// WAL 管理器
pub struct WalManager {
    /// WAL 文件路径
    wal_path: PathBuf,
    /// 当前序列号
    current_sequence: u64,
}

impl WalManager {
    /// 创建新的 WAL 管理器
    pub fn new(wal_path: PathBuf) -> Self {
        Self {
            wal_path,
            current_sequence: 0,
        }
    }

    /// 初始化 WAL
    pub async fn init(&mut self) -> Result<()> {
        // 创建 WAL 目录
        if let Some(parent) = self.wal_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        // 如果 WAL 文件存在，读取最后的序列号
        if self.wal_path.exists() {
            let content = fs::read_to_string(&self.wal_path).await?;
            let lines: Vec<&str> = content.lines().collect();
            if let Some(last_line) = lines.last()
                && let Ok(entry) = serde_json::from_str::<WalEntry>(last_line)
            {
                self.current_sequence = entry.sequence;
            }
        } else {
            // 创建空的 WAL 文件
            fs::File::create(&self.wal_path).await?;
        }

        info!("WAL 初始化完成: {:?}, sequence={}", self.wal_path, self.current_sequence);
        Ok(())
    }

    /// 写入 WAL 条目
    pub async fn write(&mut self, operation: WalOperation) -> Result<u64> {
        self.current_sequence += 1;
        let entry = WalEntry::new(self.current_sequence, operation);

        // 序列化并写入文件
        let json = serde_json::to_string(&entry)?;
        let mut file = fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.wal_path)
            .await?;

        file.write_all(format!("{}\n", json).as_bytes()).await?;
        file.sync_all().await?;

        Ok(self.current_sequence)
    }

    /// 读取所有 WAL 条目
    pub async fn read_all(&self) -> Result<Vec<WalEntry>> {
        if !self.wal_path.exists() {
            return Ok(Vec::new());
        }

        let content = fs::read_to_string(&self.wal_path).await?;
        let mut entries = Vec::new();

        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }

            match serde_json::from_str::<WalEntry>(line) {
                Ok(entry) => {
                    if entry.verify_checksum() {
                        entries.push(entry);
                    } else {
                        warn!("WAL 条目校验失败: sequence={}", entry.sequence);
                    }
                }
                Err(e) => {
                    error!("解析 WAL 条目失败: {}", e);
                }
            }
        }

        Ok(entries)
    }

    /// 清空 WAL
    pub async fn clear(&mut self) -> Result<()> {
        fs::remove_file(&self.wal_path).await?;
        fs::File::create(&self.wal_path).await?;
        self.current_sequence = 0;
        info!("WAL 已清空");
        Ok(())
    }
}

/// Chunk 校验器
pub struct ChunkVerifier {
    chunk_root: PathBuf,
}

impl ChunkVerifier {
    /// 创建新的校验器
    pub fn new(chunk_root: PathBuf) -> Self {
        Self { chunk_root }
    }

    /// 获取 chunk 实际路径（处理分层存储）
    fn get_chunk_path(&self, chunk_hash: &str) -> PathBuf {
        let prefix = &chunk_hash[..2.min(chunk_hash.len())];
        self.chunk_root.join("data").join(prefix).join(chunk_hash)
    }

    /// 验证单个 chunk
    pub async fn verify_chunk(&self, chunk_hash: &str) -> Result<bool> {
        let chunk_path = self.get_chunk_path(chunk_hash);

        if !chunk_path.exists() {
            return Ok(false);
        }

        // 读取 chunk 数据
        let data = fs::read(&chunk_path).await?;

        // 计算实际哈希
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let actual_hash = hex::encode(hasher.finalize());

        Ok(actual_hash == chunk_hash)
    }

    /// 批量验证 chunks
    pub async fn verify_chunks(&self, chunk_hashes: &[String]) -> Result<ChunkVerifyReport> {
        let mut valid = 0;
        let mut invalid = 0;
        let mut missing = 0;
        let mut corrupted_chunks = Vec::new();

        for chunk_hash in chunk_hashes {
            let chunk_path = self.get_chunk_path(chunk_hash);

            if !chunk_path.exists() {
                missing += 1;
                corrupted_chunks.push(chunk_hash.clone());
                continue;
            }

            match self.verify_chunk(chunk_hash).await {
                Ok(true) => valid += 1,
                Ok(false) => {
                    invalid += 1;
                    corrupted_chunks.push(chunk_hash.clone());
                }
                Err(e) => {
                    error!("验证 chunk 失败: {} - {}", chunk_hash, e);
                    invalid += 1;
                    corrupted_chunks.push(chunk_hash.clone());
                }
            }
        }

        Ok(ChunkVerifyReport {
            total: chunk_hashes.len(),
            valid,
            invalid,
            missing,
            corrupted_chunks,
        })
    }

    /// 扫描所有 chunks 并验证
    pub async fn scan_and_verify(&self) -> Result<ChunkVerifyReport> {
        let mut chunk_hashes = Vec::new();

        // 递归扫描 data 目录下的所有 chunk 文件
        let data_dir = self.chunk_root.join("data");
        if !data_dir.exists() {
            return Ok(ChunkVerifyReport {
                total: 0,
                valid: 0,
                invalid: 0,
                missing: 0,
                corrupted_chunks: Vec::new(),
            });
        }

        // 遍历所有前缀目录
        let mut prefix_entries = fs::read_dir(&data_dir).await?;
        while let Some(prefix_entry) = prefix_entries.next_entry().await? {
            if prefix_entry.file_type().await?.is_dir() {
                // 遍历前缀目录下的所有 chunk 文件
                let mut chunk_entries = fs::read_dir(prefix_entry.path()).await?;
                while let Some(chunk_entry) = chunk_entries.next_entry().await? {
                    if let Some(file_name) = chunk_entry.file_name().to_str() {
                        chunk_hashes.push(file_name.to_string());
                    }
                }
            }
        }

        self.verify_chunks(&chunk_hashes).await
    }
}

/// Chunk 验证报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkVerifyReport {
    /// 总数
    pub total: usize,
    /// 有效数量
    pub valid: usize,
    /// 无效数量
    pub invalid: usize,
    /// 缺失数量
    pub missing: usize,
    /// 损坏的 chunks
    pub corrupted_chunks: Vec<String>,
}

/// 孤儿 Chunk 清理器
pub struct OrphanChunkCleaner {
    chunk_root: PathBuf,
}

impl OrphanChunkCleaner {
    /// 创建新的清理器
    pub fn new(chunk_root: PathBuf) -> Self {
        Self { chunk_root }
    }

    /// 获取 chunk 实际路径（处理分层存储）
    fn get_chunk_path(&self, chunk_hash: &str) -> PathBuf {
        let prefix = &chunk_hash[..2.min(chunk_hash.len())];
        self.chunk_root.join("data").join(prefix).join(chunk_hash)
    }

    /// 检测孤儿 chunks
    pub async fn detect_orphans(
        &self,
        referenced_chunks: &HashSet<String>,
    ) -> Result<Vec<String>> {
        let mut orphans = Vec::new();

        // 递归扫描 data 目录
        let data_dir = self.chunk_root.join("data");
        if !data_dir.exists() {
            return Ok(orphans);
        }

        // 遍历所有前缀目录
        let mut prefix_entries = fs::read_dir(&data_dir).await?;
        while let Some(prefix_entry) = prefix_entries.next_entry().await? {
            if prefix_entry.file_type().await?.is_dir() {
                // 遍历前缀目录下的所有 chunk 文件
                let mut chunk_entries = fs::read_dir(prefix_entry.path()).await?;
                while let Some(chunk_entry) = chunk_entries.next_entry().await? {
                    if let Some(file_name) = chunk_entry.file_name().to_str()
                        && !referenced_chunks.contains(file_name)
                    {
                        orphans.push(file_name.to_string());
                    }
                }
            }
        }

        Ok(orphans)
    }

    /// 清理孤儿 chunks
    pub async fn clean_orphans(&self, orphan_hashes: &[String]) -> Result<CleanupReport> {
        let mut deleted = 0;
        let mut failed = Vec::new();
        let mut freed_space = 0u64;

        for chunk_hash in orphan_hashes {
            let chunk_path = self.get_chunk_path(chunk_hash);

            match fs::metadata(&chunk_path).await {
                Ok(metadata) => {
                    freed_space += metadata.len();
                    match fs::remove_file(&chunk_path).await {
                        Ok(_) => deleted += 1,
                        Err(e) => {
                            error!("删除孤儿 chunk 失败: {} - {}", chunk_hash, e);
                            failed.push(chunk_hash.clone());
                        }
                    }
                }
                Err(e) => {
                    error!("获取 chunk 元数据失败: {} - {}", chunk_hash, e);
                    failed.push(chunk_hash.clone());
                }
            }
        }

        Ok(CleanupReport {
            total: orphan_hashes.len(),
            deleted,
            failed: failed.len(),
            freed_space,
            failed_chunks: failed,
        })
    }
}

/// 清理报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupReport {
    /// 总数
    pub total: usize,
    /// 删除数量
    pub deleted: usize,
    /// 失败数量
    pub failed: usize,
    /// 释放空间（字节）
    pub freed_space: u64,
    /// 失败的 chunks
    pub failed_chunks: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_wal_entry_checksum() {
        let operation = WalOperation::CreateVersion {
            file_id: "file1".to_string(),
            version_id: "v1".to_string(),
            chunk_hashes: vec!["abc123".to_string()],
        };

        let entry = WalEntry::new(1, operation);
        assert!(entry.verify_checksum());
    }

    #[tokio::test]
    async fn test_wal_manager() {
        let temp_dir = TempDir::new().unwrap();
        let wal_path = temp_dir.path().join("test.wal");

        let mut manager = WalManager::new(wal_path);
        manager.init().await.unwrap();

        // 写入操作
        let operation = WalOperation::CreateVersion {
            file_id: "file1".to_string(),
            version_id: "v1".to_string(),
            chunk_hashes: vec!["abc123".to_string()],
        };

        let seq = manager.write(operation.clone()).await.unwrap();
        assert_eq!(seq, 1);

        // 读取所有条目
        let entries = manager.read_all().await.unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].operation, operation);
    }

    #[tokio::test]
    async fn test_chunk_verifier() {
        let temp_dir = TempDir::new().unwrap();
        let chunk_root = temp_dir.path().to_path_buf();

        // 创建测试 chunk（使用分层存储）
        let data = b"test data";
        let mut hasher = Sha256::new();
        hasher.update(data);
        let hash = hex::encode(hasher.finalize());

        let prefix = &hash[..2];
        let data_dir = chunk_root.join("data").join(prefix);
        fs::create_dir_all(&data_dir).await.unwrap();

        let chunk_path = data_dir.join(&hash);
        fs::write(&chunk_path, data).await.unwrap();

        // 验证
        let verifier = ChunkVerifier::new(chunk_root);
        let valid = verifier.verify_chunk(&hash).await.unwrap();
        assert!(valid);
    }

    #[tokio::test]
    async fn test_orphan_detection() {
        let temp_dir = TempDir::new().unwrap();
        let chunk_root = temp_dir.path().to_path_buf();

        // 创建分层存储目录
        let data_dir = chunk_root.join("data").join("ab");
        fs::create_dir_all(&data_dir).await.unwrap();

        // 创建一些 chunks
        fs::write(data_dir.join("chunk1"), b"data1")
            .await
            .unwrap();
        fs::write(data_dir.join("chunk2"), b"data2")
            .await
            .unwrap();
        fs::write(data_dir.join("chunk3"), b"data3")
            .await
            .unwrap();

        // 只有 chunk1 和 chunk2 被引用
        let mut referenced = HashSet::new();
        referenced.insert("chunk1".to_string());
        referenced.insert("chunk2".to_string());

        let cleaner = OrphanChunkCleaner::new(chunk_root);
        let orphans = cleaner.detect_orphans(&referenced).await.unwrap();

        assert_eq!(orphans.len(), 1);
        assert!(orphans.contains(&"chunk3".to_string()));
    }

    #[tokio::test]
    async fn test_cleanup_orphans() {
        let temp_dir = TempDir::new().unwrap();
        let chunk_root = temp_dir.path().to_path_buf();

        // 创建分层存储目录（确保使用正确的前缀）
        let orphan_name = "orphan1";
        let prefix = &orphan_name[..2.min(orphan_name.len())];
        let data_dir = chunk_root.join("data").join(prefix);
        fs::create_dir_all(&data_dir).await.unwrap();

        // 创建孤儿 chunk
        fs::write(data_dir.join(orphan_name), b"data1")
            .await
            .unwrap();

        let cleaner = OrphanChunkCleaner::new(chunk_root.clone());
        let report = cleaner
            .clean_orphans(&[orphan_name.to_string()])
            .await
            .unwrap();

        assert_eq!(report.deleted, 1);
        assert_eq!(report.failed, 0);
        assert!(!data_dir.join(orphan_name).exists());
    }
}
