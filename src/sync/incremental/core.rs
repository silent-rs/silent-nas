// 增量同步模块
// 实现基于块的文件差异检测和同步

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use tracing::{debug, info};

/// 默认块大小: 64KB
#[allow(dead_code)]
pub const DEFAULT_CHUNK_SIZE: usize = 64 * 1024;

/// 文件块信息
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChunkInfo {
    /// 块索引
    pub index: usize,
    /// 块起始偏移
    pub offset: u64,
    /// 块大小
    pub size: usize,
    /// 块内容的SHA256哈希
    pub hash: String,
    /// 弱哈希（快速滚动哈希，用于快速匹配）
    pub weak_hash: u32,
}

/// 文件块签名（用于差异检测）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSignature {
    /// 文件ID
    pub file_id: String,
    /// 文件总大小
    pub file_size: u64,
    /// 块大小
    pub chunk_size: usize,
    /// 文件整体SHA256哈希
    pub file_hash: String,
    /// 所有块的信息
    pub chunks: Vec<ChunkInfo>,
}

/// 差异块（需要传输的数据）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeltaChunk {
    /// 块索引
    pub index: usize,
    /// 块偏移
    pub offset: u64,
    /// 块数据
    pub data: Vec<u8>,
}

/// 同步差异信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDelta {
    /// 文件ID
    pub file_id: String,
    /// 源文件哈希
    pub source_hash: String,
    /// 目标文件哈希
    pub target_hash: String,
    /// 需要传输的块
    pub chunks: Vec<DeltaChunk>,
    /// 总块数
    pub total_chunks: usize,
    /// 需要更新的块数
    pub changed_chunks: usize,
}

/// 增量同步管理器
pub struct IncrementalSyncManager {
    /// 块大小
    chunk_size: usize,
}

impl IncrementalSyncManager {
    /// 创建新的增量同步管理器
    pub fn new(chunk_size: usize) -> Self {
        Self { chunk_size }
    }

    /// 使用默认块大小创建管理器
    #[allow(dead_code)]
    pub fn default() -> Self {
        Self::new(DEFAULT_CHUNK_SIZE)
    }

    /// 计算文件签名
    pub fn calculate_signature(&self, file_id: &str, data: &[u8]) -> Result<FileSignature> {
        let file_size = data.len() as u64;
        let mut chunks = Vec::new();

        // 计算文件整体哈希
        let file_hash = format!("{:x}", Sha256::digest(data));

        // 分块并计算每个块的哈希
        for (index, chunk_data) in data.chunks(self.chunk_size).enumerate() {
            let offset = (index * self.chunk_size) as u64;
            let size = chunk_data.len();

            // 强哈希（SHA256）
            let hash = format!("{:x}", Sha256::digest(chunk_data));

            // 弱哈希（简单的滚动哈希）
            let weak_hash = self.calculate_weak_hash(chunk_data);

            chunks.push(ChunkInfo {
                index,
                offset,
                size,
                hash,
                weak_hash,
            });
        }

        Ok(FileSignature {
            file_id: file_id.to_string(),
            file_size,
            chunk_size: self.chunk_size,
            file_hash,
            chunks,
        })
    }

    /// 计算弱哈希（Adler-32类似算法）
    fn calculate_weak_hash(&self, data: &[u8]) -> u32 {
        let mut a: u32 = 1;
        let mut b: u32 = 0;
        const MOD_ADLER: u32 = 65521;

        for &byte in data {
            a = (a + byte as u32) % MOD_ADLER;
            b = (b + a) % MOD_ADLER;
        }

        (b << 16) | a
    }

    /// 比较两个签名，计算差异
    pub fn calculate_delta(
        &self,
        source_sig: &FileSignature,
        target_sig: &FileSignature,
    ) -> Result<Option<SyncDelta>> {
        // 如果文件哈希相同，无需同步
        if source_sig.file_hash == target_sig.file_hash {
            debug!("文件哈希相同，无需同步: file_id={}", source_sig.file_id);
            return Ok(None);
        }

        // 构建目标块的哈希映射（用于快速查找）
        let mut target_chunks: HashMap<String, &ChunkInfo> = HashMap::new();
        for chunk in &target_sig.chunks {
            target_chunks.insert(chunk.hash.clone(), chunk);
        }

        // 找出需要更新的块
        let mut changed_indices = Vec::new();
        for source_chunk in &source_sig.chunks {
            // 检查目标是否有相同哈希的块
            if !target_chunks.contains_key(&source_chunk.hash) {
                changed_indices.push(source_chunk.index);
            }
        }

        // 如果源文件比目标文件大，需要添加新块
        if source_sig.chunks.len() > target_sig.chunks.len() {
            for i in target_sig.chunks.len()..source_sig.chunks.len() {
                changed_indices.push(i);
            }
        }

        info!(
            "文件差异检测完成: file_id={}, 总块数={}, 变更块数={}",
            source_sig.file_id,
            source_sig.chunks.len(),
            changed_indices.len()
        );

        Ok(Some(SyncDelta {
            file_id: source_sig.file_id.clone(),
            source_hash: source_sig.file_hash.clone(),
            target_hash: target_sig.file_hash.clone(),
            chunks: Vec::new(), // 实际数据需要后续填充
            total_chunks: source_sig.chunks.len(),
            changed_chunks: changed_indices.len(),
        }))
    }

    /// 从完整文件数据中提取差异块
    pub fn extract_delta_chunks(
        &self,
        data: &[u8],
        delta: &SyncDelta,
        source_sig: &FileSignature,
    ) -> Result<Vec<DeltaChunk>> {
        let mut chunks = Vec::new();

        // 构建目标块的哈希集合
        let target_hashes: std::collections::HashSet<String> = std::collections::HashSet::new();

        // 提取需要传输的块
        for chunk_info in &source_sig.chunks {
            if !target_hashes.contains(&chunk_info.hash) {
                let start = chunk_info.offset as usize;
                let end = (start + chunk_info.size).min(data.len());

                if start < data.len() {
                    chunks.push(DeltaChunk {
                        index: chunk_info.index,
                        offset: chunk_info.offset,
                        data: data[start..end].to_vec(),
                    });
                }
            }
        }

        info!(
            "提取差异块完成: file_id={}, 提取块数={}",
            delta.file_id,
            chunks.len()
        );

        Ok(chunks)
    }

    /// 应用差异块到目标文件
    pub fn apply_delta(&self, target_data: &[u8], delta_chunks: &[DeltaChunk]) -> Result<Vec<u8>> {
        // 计算最终文件大小
        let mut max_offset = target_data.len();
        for chunk in delta_chunks {
            let end = (chunk.offset as usize + chunk.data.len()).max(max_offset);
            max_offset = end;
        }

        // 创建新文件缓冲区
        let mut result = target_data.to_vec();
        result.resize(max_offset, 0);

        // 应用差异块
        for chunk in delta_chunks {
            let start = chunk.offset as usize;
            let end = start + chunk.data.len();

            if end > result.len() {
                result.resize(end, 0);
            }

            result[start..end].copy_from_slice(&chunk.data);
        }

        info!("差异块应用完成: 应用了 {} 个块", delta_chunks.len());

        Ok(result)
    }

    /// 验证应用后的文件哈希
    pub fn verify_hash(&self, data: &[u8], expected_hash: &str) -> bool {
        let actual_hash = format!("{:x}", Sha256::digest(data));
        actual_hash == expected_hash
    }

    /// 计算传输节省的比例
    pub fn calculate_savings(&self, file_size: u64, delta: &SyncDelta) -> (u64, u64, f64) {
        let transferred = (delta.changed_chunks * self.chunk_size) as u64;
        let saved = file_size.saturating_sub(transferred);
        let savings_percent = if file_size > 0 {
            (saved as f64 / file_size as f64) * 100.0
        } else {
            0.0
        };

        (transferred, saved, savings_percent)
    }
}

/// 快速差异检测（仅比较文件哈希）
#[allow(dead_code)]
pub fn quick_diff_check(local_hash: &str, remote_hash: &str) -> bool {
    local_hash != remote_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_calculate_signature() {
        let manager = IncrementalSyncManager::default();
        let data = b"Hello, World! This is a test file.";
        let sig = manager.calculate_signature("test_file", data).unwrap();

        assert_eq!(sig.file_id, "test_file");
        assert_eq!(sig.file_size, data.len() as u64);
        assert!(!sig.chunks.is_empty());
        assert!(!sig.file_hash.is_empty());
    }

    #[test]
    fn test_weak_hash() {
        let manager = IncrementalSyncManager::default();
        let data1 = b"test data";
        let data2 = b"test data";
        let data3 = b"different";

        let hash1 = manager.calculate_weak_hash(data1);
        let hash2 = manager.calculate_weak_hash(data2);
        let hash3 = manager.calculate_weak_hash(data3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_identical_files() {
        let manager = IncrementalSyncManager::new(1024);
        let data = b"Same content for both files";

        let sig1 = manager.calculate_signature("file1", data).unwrap();
        let sig2 = manager.calculate_signature("file2", data).unwrap();

        let delta = manager.calculate_delta(&sig1, &sig2).unwrap();
        assert!(delta.is_none());
    }

    #[test]
    fn test_different_files() {
        let manager = IncrementalSyncManager::new(10);
        let data1 = b"Original content here";
        let data2 = b"Modified content here";

        let sig1 = manager.calculate_signature("file1", data1).unwrap();
        let sig2 = manager.calculate_signature("file2", data2).unwrap();

        let delta = manager.calculate_delta(&sig1, &sig2).unwrap();
        assert!(delta.is_some());

        let delta = delta.unwrap();
        assert!(delta.changed_chunks > 0);
    }

    #[test]
    fn test_apply_delta() {
        let manager = IncrementalSyncManager::new(10);
        let original = b"0123456789ABCDEFGHIJ";
        let _modified = b"0123456789XYZEFGHIJ";

        // 创建差异块
        let chunks = vec![DeltaChunk {
            index: 1,
            offset: 10,
            data: b"XYZ".to_vec(),
        }];

        let result = manager.apply_delta(original, &chunks).unwrap();
        assert_eq!(&result[10..13], b"XYZ");
    }

    #[test]
    fn test_verify_hash() {
        let manager = IncrementalSyncManager::default();
        let data = b"Test data for hashing";
        let hash = format!("{:x}", Sha256::digest(data));

        assert!(manager.verify_hash(data, &hash));
        assert!(!manager.verify_hash(data, "invalid_hash"));
    }

    #[test]
    fn test_calculate_savings() {
        let manager = IncrementalSyncManager::new(1024);
        let delta = SyncDelta {
            file_id: "test".to_string(),
            source_hash: "hash1".to_string(),
            target_hash: "hash2".to_string(),
            chunks: vec![],
            total_chunks: 10,
            changed_chunks: 2,
        };

        let (transferred, saved, percent) = manager.calculate_savings(10240, &delta);
        assert_eq!(transferred, 2048); // 2 chunks * 1024
        assert_eq!(saved, 8192); // 10240 - 2048
        assert!((percent - 80.0).abs() < 0.01);
    }

    #[test]
    fn test_quick_diff_check() {
        assert!(quick_diff_check("hash1", "hash2"));
        assert!(!quick_diff_check("same", "same"));
    }
}
