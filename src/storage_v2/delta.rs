//! 文件差异生成与应用模块
//!
//! 该模块实现增量更新的差异生成和应用功能

use crate::error::{NasError, Result};
use crate::storage_v2::{FileDelta, IncrementalConfig, RabinKarpChunker};
use chrono::Local;
use sha2::Digest;
use std::collections::HashMap;

/// 差异生成器
pub struct DeltaGenerator {
    #[allow(dead_code)]
    config: IncrementalConfig,
    chunker: RabinKarpChunker,
}

impl DeltaGenerator {
    pub fn new(config: IncrementalConfig) -> Self {
        let chunker = RabinKarpChunker::new(config.clone());
        Self { config, chunker }
    }

    /// 生成文件差异
    /// base_data: 基础版本数据（空 Vec 表示从空文件开始）
    /// new_data: 新版本数据
    /// file_id: 文件ID
    /// base_version_id: 基础版本ID（空字符串表示从空文件开始）
    pub fn generate_delta(
        &mut self,
        _base_data: &[u8],
        new_data: &[u8],
        file_id: &str,
        base_version_id: &str,
    ) -> Result<FileDelta> {
        // 如果新数据为空，返回空差异
        if new_data.is_empty() {
            return Ok(FileDelta {
                file_id: file_id.to_string(),
                base_version_id: base_version_id.to_string(),
                new_version_id: generate_version_id(),
                chunks: Vec::new(),
                created_at: Local::now().naive_local(),
            });
        }

        // 对新数据分块
        let chunks = self
            .chunker
            .chunk_data(new_data)
            .map_err(|e| NasError::Other(format!("分块失败: {}", e)))?;

        Ok(FileDelta {
            file_id: file_id.to_string(),
            base_version_id: base_version_id.to_string(),
            new_version_id: generate_version_id(),
            chunks,
            created_at: Local::now().naive_local(),
        })
    }

    /// 生成完整版本的差异（从空文件开始）
    pub fn generate_full_delta(&mut self, data: &[u8], file_id: &str) -> Result<FileDelta> {
        self.generate_delta(&[], data, file_id, "")
    }

    /// 比较两个版本的差异
    pub fn compare_versions(
        &mut self,
        old_data: &[u8],
        new_data: &[u8],
        file_id: &str,
    ) -> Result<FileDelta> {
        self.generate_delta(old_data, new_data, file_id, "previous")
    }
}

/// 差异应用器
pub struct DeltaApplier {
    #[allow(dead_code)]
    config: IncrementalConfig,
    /// 块存储缓存：chunk_id -> 块数据
    block_cache: HashMap<String, Vec<u8>>,
}

impl DeltaApplier {
    pub fn new(config: IncrementalConfig) -> Self {
        Self {
            config,
            block_cache: HashMap::new(),
        }
    }

    /// 从差异重建文件数据
    /// base_data: 基础版本数据（如果 base_version_id 为空，则忽略此参数）
    /// delta: 文件差异
    /// chunk_reader: 读取块的回调函数
    pub fn apply_delta<F>(
        &mut self,
        base_data: Option<&[u8]>,
        delta: &FileDelta,
        mut chunk_reader: F,
    ) -> Result<Vec<u8>>
    where
        F: FnMut(&str) -> Result<Vec<u8>>,
    {
        // 如果没有基础数据且base_version_id为空，创建空文件
        if delta.chunks.is_empty() {
            return Ok(Vec::new());
        }

        // 如果base_version_id为空，使用空数据作为基础
        let base_data = if delta.base_version_id.is_empty() {
            Vec::new()
        } else {
            base_data.unwrap_or(&[]).to_vec()
        };

        // 重建文件：应用所有分块
        let mut result = Vec::new();
        let mut base_pos = 0usize;

        for chunk in &delta.chunks {
            // 复制基础数据中到当前块偏移量的部分
            if chunk.offset > base_pos {
                let copy_len = chunk.offset - base_pos;
                if base_pos + copy_len <= base_data.len() {
                    result.extend_from_slice(&base_data[base_pos..base_pos + copy_len]);
                }
                base_pos = chunk.offset;
            }

            // 读取并添加新块数据
            if let Some(cached_chunk) = self.block_cache.get(&chunk.chunk_id) {
                result.extend_from_slice(cached_chunk);
            } else {
                let chunk_data = chunk_reader(&chunk.chunk_id)?;
                self.block_cache
                    .insert(chunk.chunk_id.clone(), chunk_data.clone());
                result.extend_from_slice(&chunk_data);
            }
        }

        // 复制基础数据中剩余的部分
        if base_pos < base_data.len() {
            result.extend_from_slice(&base_data[base_pos..]);
        }

        Ok(result)
    }

    /// 验证差异的完整性
    pub fn verify_delta<F>(&mut self, delta: &FileDelta, mut chunk_reader: F) -> Result<bool>
    where
        F: FnMut(&str) -> Result<Vec<u8>>,
    {
        for chunk in &delta.chunks {
            // 读取块数据
            let chunk_data = if let Some(cached) = self.block_cache.get(&chunk.chunk_id) {
                cached.clone()
            } else {
                let data = chunk_reader(&chunk.chunk_id)?;
                self.block_cache
                    .insert(chunk.chunk_id.clone(), data.clone());
                data
            };

            // 验证块大小
            if chunk_data.len() != chunk.size {
                return Ok(false);
            }

            // 验证强哈希
            let mut hasher = sha2::Sha256::new();
            hasher.update(&chunk_data);
            let calculated_hash = hex::encode(hasher.finalize());
            if calculated_hash != chunk.strong_hash {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// 清理缓存
    pub fn clear_cache(&mut self) {
        self.block_cache.clear();
    }

    /// 获取缓存信息
    pub fn cache_info(&self) -> (usize, usize) {
        let count = self.block_cache.len();
        let size: usize = self.block_cache.values().map(|v| v.len()).sum();
        (count, size)
    }
}

/// 生成版本ID
fn generate_version_id() -> String {
    format!("v_{}", scru128::new())
}

/// 差异统计信息
#[derive(Debug, Clone)]
pub struct DeltaStats {
    /// 原始数据大小
    pub original_size: u64,
    /// 分块数量
    pub chunk_count: usize,
    /// 总块大小
    pub total_chunk_size: u64,
    /// 平均块大小
    pub avg_chunk_size: f64,
    /// 最小块大小
    pub min_chunk_size: usize,
    /// 最大块大小
    pub max_chunk_size: usize,
}

impl FileDelta {
    /// 获取差异统计信息
    pub fn get_stats(&self) -> DeltaStats {
        if self.chunks.is_empty() {
            return DeltaStats {
                original_size: 0,
                chunk_count: 0,
                total_chunk_size: 0,
                avg_chunk_size: 0.0,
                min_chunk_size: 0,
                max_chunk_size: 0,
            };
        }

        let sizes: Vec<usize> = self.chunks.iter().map(|c| c.size).collect();
        let min_size = sizes.iter().min().copied().unwrap_or(0);
        let max_size = sizes.iter().max().copied().unwrap_or(0);
        let total_size: u64 = sizes.iter().sum::<usize>() as u64;
        let avg_size = total_size as f64 / self.chunks.len() as f64;

        DeltaStats {
            original_size: total_size,
            chunk_count: self.chunks.len(),
            total_chunk_size: total_size,
            avg_chunk_size: avg_size,
            min_chunk_size: min_size,
            max_chunk_size: max_size,
        }
    }

    /// 检查差异是否为空（没有变化）
    pub fn is_empty(&self) -> bool {
        self.chunks.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_generator() -> DeltaGenerator {
        let config = IncrementalConfig::default();
        DeltaGenerator::new(config)
    }

    fn create_test_applier() -> DeltaApplier {
        let config = IncrementalConfig::default();
        DeltaApplier::new(config)
    }

    #[test]
    fn test_generate_full_delta() {
        let mut generator = create_test_generator();
        let data = b"Hello, World! This is a test file.";

        let delta = generator.generate_full_delta(data, "test_file").unwrap();

        assert_eq!(delta.file_id, "test_file");
        assert!(delta.base_version_id.is_empty());
        assert!(!delta.new_version_id.is_empty());
        assert!(!delta.chunks.is_empty());
    }

    #[test]
    fn test_generate_delta() {
        let mut generator = create_test_generator();
        let base_data = b"Hello, World!";
        let new_data = b"Hello, World! This is a test.";

        let delta = generator
            .generate_delta(base_data, new_data, "test_file", "v_1")
            .unwrap();

        assert_eq!(delta.file_id, "test_file");
        assert_eq!(delta.base_version_id, "v_1");
        assert!(!delta.new_version_id.is_empty());
    }

    #[test]
    fn test_apply_delta() {
        let mut applier = create_test_applier();
        let base_data = b"Hello, World!";
        let new_data = b"Hello, World! This is a test.";

        let mut generator = create_test_generator();
        let delta = generator
            .generate_delta(base_data, new_data, "test_file", "v_1")
            .unwrap();

        // 模拟块读取器
        let mut chunks: HashMap<String, Vec<u8>> = HashMap::new();
        for chunk in &delta.chunks {
            let chunk_data = &new_data[chunk.offset..chunk.offset + chunk.size];
            chunks.insert(chunk.chunk_id.clone(), chunk_data.to_vec());
        }

        let chunk_reader = |chunk_id: &str| -> Result<Vec<u8>> {
            Ok(chunks.get(chunk_id).cloned().unwrap_or_default())
        };

        let result = applier
            .apply_delta(Some(base_data), &delta, chunk_reader)
            .unwrap();

        assert_eq!(result, new_data);
    }

    #[test]
    fn test_verify_delta() {
        let mut applier = create_test_applier();
        let data = b"Hello, World! This is a test.";

        let mut generator = create_test_generator();
        let delta = generator.generate_full_delta(data, "test_file").unwrap();

        // 创建块数据
        let mut chunks: HashMap<String, Vec<u8>> = HashMap::new();
        for chunk in &delta.chunks {
            let chunk_data = &data[chunk.offset..chunk.offset + chunk.size];
            chunks.insert(chunk.chunk_id.clone(), chunk_data.to_vec());
        }

        let chunk_reader = |chunk_id: &str| -> Result<Vec<u8>> {
            Ok(chunks.get(chunk_id).cloned().unwrap_or_default())
        };

        assert!(applier.verify_delta(&delta, chunk_reader).unwrap());
    }

    #[test]
    fn test_delta_stats() {
        let data = b"Hello, World! This is a test file with some content.";
        let mut generator = create_test_generator();
        let delta = generator.generate_full_delta(data, "test_file").unwrap();

        let stats = delta.get_stats();

        assert_eq!(stats.chunk_count, delta.chunks.len());
        assert!(stats.min_chunk_size > 0);
        assert!(stats.max_chunk_size >= stats.min_chunk_size);
        assert!(stats.avg_chunk_size > 0.0);
    }

    #[test]
    fn test_is_empty() {
        let mut generator = create_test_generator();
        let empty_delta = generator
            .generate_delta(&[], &[], "test_file", "v_1")
            .unwrap();

        assert!(empty_delta.is_empty());
    }
}
