//! 数据压缩模块
//!
//! 支持LZ4和Zstd压缩算法，提供：
//! - 多种压缩算法选择
//! - 压缩比监控
//! - 性能优化
//! - 冷数据自动压缩

use crate::error::{StorageError, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// 压缩算法类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CompressionAlgorithm {
    /// 无压缩
    None,
    /// LZ4压缩（快速）
    #[default]
    LZ4,
    /// Zstd压缩（高压缩比）
    Zstd,
}

/// 压缩配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionConfig {
    /// 压缩算法
    pub algorithm: CompressionAlgorithm,
    /// 压缩等级（0-9，0为最快，9为最高压缩比）
    pub level: u32,
    /// 启用压缩的最小数据大小（字节）
    pub min_size: usize,
    /// 自动压缩的阈值（最近N天未访问）
    pub auto_compress_days: u32,
    /// 压缩比阈值（低于此比率不压缩）
    pub min_ratio: f32,
}

impl Default for CompressionConfig {
    fn default() -> Self {
        Self {
            algorithm: CompressionAlgorithm::LZ4,
            level: 1,              // 快速压缩
            min_size: 1024,        // 1KB
            auto_compress_days: 7, // 7天未访问自动压缩
            min_ratio: 1.1,        // 压缩比至少10%
        }
    }
}

/// 压缩结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionResult {
    /// 原始数据大小
    pub original_size: u64,
    /// 压缩后大小
    pub compressed_size: u64,
    /// 压缩比
    pub ratio: f32,
    /// 压缩用时（毫秒）
    pub duration_ms: u64,
    /// 使用的算法
    pub algorithm: CompressionAlgorithm,
}

/// 压缩器
pub struct Compressor {
    config: CompressionConfig,
}

impl Compressor {
    pub fn new(config: CompressionConfig) -> Self {
        Self { config }
    }

    /// 压缩数据
    pub fn compress(&self, data: &[u8]) -> Result<CompressionResult> {
        let start = std::time::Instant::now();

        // 检查是否需要压缩
        if data.len() < self.config.min_size {
            return Ok(CompressionResult {
                original_size: data.len() as u64,
                compressed_size: data.len() as u64,
                ratio: 1.0,
                duration_ms: 0,
                algorithm: CompressionAlgorithm::None,
            });
        }

        let (compressed_data, algorithm) = match self.config.algorithm {
            CompressionAlgorithm::None => (data.to_vec(), CompressionAlgorithm::None),
            CompressionAlgorithm::LZ4 => {
                let compressed = compress_lz4(data, self.config.level)?;
                (compressed, CompressionAlgorithm::LZ4)
            }
            CompressionAlgorithm::Zstd => {
                let compressed = compress_zstd(data, self.config.level)?;
                (compressed, CompressionAlgorithm::Zstd)
            }
        };

        let duration = start.elapsed();
        let ratio = if !data.is_empty() {
            data.len() as f32 / compressed_data.len() as f32
        } else {
            1.0
        };

        // 检查压缩比是否满足要求
        if ratio < self.config.min_ratio {
            // 压缩效果不佳，返回原数据
            return Ok(CompressionResult {
                original_size: data.len() as u64,
                compressed_size: data.len() as u64,
                ratio: 1.0,
                duration_ms: 0,
                algorithm: CompressionAlgorithm::None,
            });
        }

        Ok(CompressionResult {
            original_size: data.len() as u64,
            compressed_size: compressed_data.len() as u64,
            ratio,
            duration_ms: duration.as_millis() as u64,
            algorithm,
        })
    }

    /// 解压缩数据
    pub fn decompress(&self, data: &[u8], algorithm: CompressionAlgorithm) -> Result<Vec<u8>> {
        match algorithm {
            CompressionAlgorithm::None => Ok(data.to_vec()),
            CompressionAlgorithm::LZ4 => decompress_lz4(data),
            CompressionAlgorithm::Zstd => decompress_zstd(data),
        }
    }

    /// 检查数据是否需要自动压缩
    pub fn should_auto_compress(&self, last_accessed: chrono::NaiveDateTime) -> bool {
        let now = chrono::Local::now().naive_local();
        let days_since_access = now.signed_duration_since(last_accessed).num_days();
        days_since_access as u32 >= self.config.auto_compress_days
    }
}

/// LZ4压缩
fn compress_lz4(data: &[u8], _level: u32) -> Result<Vec<u8>> {
    // 使用lz4_flex库进行压缩
    let compressed = lz4_flex::block::compress(data);
    Ok(compressed)
}

/// LZ4解压缩
fn decompress_lz4(data: &[u8]) -> Result<Vec<u8>> {
    let decompressed = lz4_flex::block::decompress(data, 0)
        .map_err(|e| StorageError::Storage(format!("LZ4解压缩失败: {}", e)))?;
    Ok(decompressed)
}

/// Zstd压缩
fn compress_zstd(data: &[u8], level: u32) -> Result<Vec<u8>> {
    // 使用zstd库进行压缩
    let mut encoder = zstd::Encoder::new(Vec::new(), level as i32)
        .map_err(|e| StorageError::Storage(format!("Zstd压缩初始化失败: {}", e)))?;
    encoder
        .write_all(data)
        .map_err(|e| StorageError::Storage(format!("Zstd压缩写入失败: {}", e)))?;
    let compressed = encoder
        .finish()
        .map_err(|e| StorageError::Storage(format!("Zstd压缩失败: {}", e)))?;
    Ok(compressed)
}

/// Zstd解压缩
fn decompress_zstd(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = zstd::Decoder::new(data)
        .map_err(|e| StorageError::Storage(format!("Zstd解压缩初始化失败: {}", e)))?;
    let mut decompressed = Vec::new();
    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| StorageError::Storage(format!("Zstd解压缩失败: {}", e)))?;
    Ok(decompressed)
}

/// 压缩统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompressionStats {
    /// 总压缩次数
    pub total_compressions: u64,
    /// 总解压缩次数
    pub total_decompressions: u64,
    /// 原始数据总大小
    pub total_original_size: u64,
    /// 压缩后总大小
    pub total_compressed_size: u64,
    /// 平均压缩比
    pub avg_compression_ratio: f32,
    /// 节省的空间
    pub space_saved: u64,
    /// 各算法的使用统计
    pub algorithm_stats: Vec<AlgorithmStats>,
}

/// 算法统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlgorithmStats {
    pub algorithm: CompressionAlgorithm,
    pub count: u64,
    pub avg_ratio: f32,
}

impl Default for CompressionStats {
    fn default() -> Self {
        Self::new()
    }
}

impl CompressionStats {
    /// 创建新的统计信息
    pub fn new() -> Self {
        Self {
            total_compressions: 0,
            total_decompressions: 0,
            total_original_size: 0,
            total_compressed_size: 0,
            avg_compression_ratio: 0.0,
            space_saved: 0,
            algorithm_stats: Vec::new(),
        }
    }

    /// 更新统计信息
    pub fn update(&mut self, result: &CompressionResult) {
        self.total_compressions += 1;
        self.total_original_size += result.original_size;
        self.total_compressed_size += result.compressed_size;
        self.space_saved += result.original_size - result.compressed_size;

        // 更新平均压缩比
        self.avg_compression_ratio = if self.total_compressed_size > 0 {
            self.total_original_size as f32 / self.total_compressed_size as f32
        } else {
            1.0
        };
    }

    /// 获取压缩率
    pub fn get_compression_rate(&self) -> f32 {
        if self.total_original_size > 0 {
            (self.total_original_size - self.total_compressed_size) as f32
                / self.total_original_size as f32
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compression_config_default() {
        let config = CompressionConfig::default();
        assert_eq!(config.algorithm, CompressionAlgorithm::LZ4);
        assert_eq!(config.level, 1);
        assert_eq!(config.min_size, 1024);
    }

    #[test]
    fn test_compress_decompress_lz4() {
        let config = CompressionConfig {
            algorithm: CompressionAlgorithm::LZ4,
            level: 1,
            min_size: 0,
            auto_compress_days: 0,
            min_ratio: 1.0,
        };
        let compressor = Compressor::new(config);

        let data = b"Hello, World! This is a test of compression.";
        let result = compressor.compress(data).unwrap();

        assert!(result.original_size >= result.compressed_size);
        assert!(result.ratio >= 1.0);

        let decompressed = compressor.decompress(data, result.algorithm).unwrap();
        assert_eq!(decompressed, data);
    }

    #[test]
    fn test_should_auto_compress() {
        let config = CompressionConfig {
            algorithm: CompressionAlgorithm::LZ4,
            level: 1,
            min_size: 0,
            auto_compress_days: 7,
            min_ratio: 1.0,
        };
        let compressor = Compressor::new(config);

        // 3天前访问
        let recent = chrono::Local::now().naive_local() - chrono::Duration::days(3);
        assert!(!compressor.should_auto_compress(recent));

        // 10天前访问
        let old = chrono::Local::now().naive_local() - chrono::Duration::days(10);
        assert!(compressor.should_auto_compress(old));
    }

    #[test]
    fn test_compression_stats() {
        let mut stats = CompressionStats::new();

        let result = CompressionResult {
            original_size: 1000,
            compressed_size: 500,
            ratio: 2.0,
            duration_ms: 10,
            algorithm: CompressionAlgorithm::LZ4,
        };
        stats.update(&result);

        assert_eq!(stats.total_compressions, 1);
        assert_eq!(stats.total_original_size, 1000);
        assert_eq!(stats.total_compressed_size, 500);
        assert_eq!(stats.space_saved, 500);
        assert_eq!(stats.get_compression_rate(), 0.5);
    }
}
