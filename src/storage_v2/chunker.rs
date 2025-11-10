//! 滚动哈希分块器
//!
//! 实现基于Rabin-Karp的内容定义分块算法，支持：
//! - 滚动哈希计算
//! - 弱哈希 + 强哈希双校验
//! - 边界检测

use crate::error::{NasError, Result};
use crate::storage_v2::{ChunkInfo, ChunkerType, IncrementalConfig};
use sha2::{Digest, Sha256};
use std::io::{self, Read};

/// Rabin-Karp 滚动哈希分块器
pub struct RabinKarpChunker {
    config: IncrementalConfig,
    /// 当前弱哈希值
    weak_hash: u32,
    /// 滑动窗口
    window: Vec<u8>,
    /// 窗口大小（通常为48字节）
    window_size: usize,
    /// 窗口中字节的幂次和
    hash_power: u64,
}

impl RabinKarpChunker {
    pub fn new(config: IncrementalConfig) -> Self {
        let window_size = 48; // 常用窗口大小
        let hash_power = calculate_power(config.rabin_poly, window_size - 1);

        Self {
            config,
            weak_hash: 0,
            window: Vec::with_capacity(window_size),
            window_size,
            hash_power,
        }
    }

    /// 计算块的边界检查
    fn is_chunk_boundary(&self, weak_hash: u32, bytes_processed: usize) -> bool {
        // 弱哈希值满足边界条件且已达到最小分块大小
        (weak_hash as usize) % self.config.weak_hash_mod == 0
            && bytes_processed >= self.config.min_chunk_size
    }

    /// 滚动计算哈希值
    fn roll_hash(&self, outgoing: u8, incoming: u8, old_hash: u32) -> u32 {
        // (old_hash - outgoing * base^window_size-1) * base + incoming
        let old_hash_u64 = old_hash as u64;
        let outgoing_u64 = outgoing as u64;
        let incoming_u64 = incoming as u64;

        let new_hash = (old_hash_u64 + self.config.rabin_poly - outgoing_u64 * self.hash_power)
            * self.config.rabin_poly as u64
            + incoming_u64;

        (new_hash % u32::MAX as u64) as u32
    }

    /// 计算弱哈希
    fn calculate_weak_hash(&self, data: &[u8]) -> u32 {
        let mut hash: u64 = 0;
        for &byte in data {
            hash = hash
                .wrapping_mul(self.config.rabin_poly as u64)
                .wrapping_add(byte as u64);
        }
        (hash % u32::MAX as u64) as u32
    }

    /// 计算强哈希（SHA-256）
    fn calculate_strong_hash(&self, data: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(data);
        hex::encode(hasher.finalize())
    }

    /// 生成分块
    pub fn chunk_data(&mut self, data: &[u8]) -> Result<Vec<ChunkInfo>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut bytes_processed = 0;

        // 初始化：填充窗口
        self.window.clear();
        let init_size = std::cmp::min(self.window_size, data.len());
        self.window.extend_from_slice(&data[..init_size]);

        if !self.window.is_empty() {
            self.weak_hash = self.calculate_weak_hash(&self.window);
            bytes_processed = init_size;
        }

        let mut i = init_size;
        while i < data.len() {
            // 检查是否满足分块边界
            if self.is_chunk_boundary(self.weak_hash, bytes_processed) {
                // 生成分块
                let chunk_data = &data[offset..i];
                let chunk = ChunkInfo {
                    chunk_id: self.calculate_strong_hash(chunk_data),
                    offset,
                    size: chunk_data.len(),
                    weak_hash: self.weak_hash,
                    strong_hash: self.calculate_strong_hash(chunk_data),
                };
                chunks.push(chunk);

                // 更新状态
                offset = i as u64;
                bytes_processed = 0;

                // 重新初始化窗口
                self.window.clear();
                let new_window_size = std::cmp::min(self.window_size, data.len() - i);
                self.window.extend_from_slice(&data[i..i + new_window_size]);

                if !self.window.is_empty() {
                    self.weak_hash = self.calculate_weak_hash(&self.window);
                    bytes_processed = new_window_size;
                } else {
                    self.weak_hash = 0;
                }

                i += new_window_size;
            } else {
                // 滑动窗口：移除最旧字节，添加新字节
                if self.window.len() == self.window_size {
                    let outgoing = self.window[0];
                    self.window.remove(0);
                }

                self.window.push(data[i]);

                if self.window.len() == self.window_size {
                    self.weak_hash = self.roll_hash(self.window[0], data[i], self.weak_hash);
                }

                bytes_processed += 1;
                i += 1;
            }
        }

        // 处理最后一个块
        if offset < data.len() as u64 {
            let remaining_data = &data[offset as usize..];
            if !remaining_data.is_empty() {
                let chunk = ChunkInfo {
                    chunk_id: self.calculate_strong_hash(remaining_data),
                    offset,
                    size: remaining_data.len(),
                    weak_hash: if self.window.is_empty() {
                        self.calculate_weak_hash(remaining_data)
                    } else {
                        self.weak_hash
                    },
                    strong_hash: self.calculate_strong_hash(remaining_data),
                };
                chunks.push(chunk);
            }
        }

        Ok(chunks)
    }
}

/// 通用分块器 trait
pub trait Chunker {
    /// 生成分块
    fn chunk(&mut self, data: &[u8]) -> Result<Vec<ChunkInfo>>;
}

/// 固定大小分块器
pub struct FixedSizeChunker {
    chunk_size: usize,
}

impl FixedSizeChunker {
    pub fn new(chunk_size: usize) -> Self {
        Self { chunk_size }
    }
}

impl Chunker for FixedSizeChunker {
    fn chunk(&mut self, data: &[u8]) -> Result<Vec<ChunkInfo>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;

        for chunk in data.chunks(self.chunk_size) {
            let mut hasher = Sha256::new();
            hasher.update(chunk);
            let strong_hash = hex::encode(hasher.finalize());

            chunks.push(ChunkInfo {
                chunk_id: strong_hash.clone(),
                offset,
                size: chunk.len(),
                weak_hash: 0, // 固定大小不需要弱哈希
                strong_hash,
            });

            offset += chunk.len() as u64;
        }

        Ok(chunks)
    }
}

/// 快速分块器（简单哈希）
pub struct FastChunker {
    min_chunk_size: usize,
    max_chunk_size: usize,
    avg_chunk_size: usize,
}

impl FastChunker {
    pub fn new(min_chunk_size: usize, max_chunk_size: usize, avg_chunk_size: usize) -> Self {
        Self {
            min_chunk_size,
            max_chunk_size,
            avg_chunk_size,
        }
    }
}

impl Chunker for FastChunker {
    fn chunk(&mut self, data: &[u8]) -> Result<Vec<ChunkInfo>> {
        let mut chunks = Vec::new();
        let mut offset = 0u64;
        let mut i = 0;

        while i < data.len() {
            let remaining = data.len() - i;
            let target_size = if remaining < self.min_chunk_size {
                remaining
            } else if i == 0 {
                // 第一个块使用平均大小
                std::cmp::min(self.avg_chunk_size, remaining)
            } else {
                // 计算下一个分块位置
                let boundary = i + self.avg_chunk_size;
                std::cmp::min(boundary, i + self.max_chunk_size)
            };

            let chunk_end = std::cmp::min(i + target_size, data.len());
            let chunk = &data[i..chunk_end];

            let mut hasher = Sha256::new();
            hasher.update(chunk);
            let strong_hash = hex::encode(hasher.finalize());

            chunks.push(ChunkInfo {
                chunk_id: strong_hash.clone(),
                offset,
                size: chunk.len(),
                weak_hash: 0,
                strong_hash,
            });

            offset += chunk.len() as u64;
            i = chunk_end;
        }

        Ok(chunks)
    }
}

/// 计算多项式的幂
fn calculate_power(base: u64, exp: usize) -> u64 {
    let mut result = 1u64;
    for _ in 0..exp {
        result = result.wrapping_mul(base);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rabinkarp_chunker_basic() {
        let config = IncrementalConfig::default();
        let mut chunker = RabinKarpChunker::new(config);

        let data = b"Hello, World! This is a test of the chunker.";
        let chunks = chunker.chunk_data(data).unwrap();

        assert!(!chunks.is_empty());
        assert!(chunks.iter().all(|c| !c.chunk_id.is_empty()));
    }

    #[test]
    fn test_fixed_size_chunker() {
        let mut chunker = FixedSizeChunker::new(8);
        let data = b"Hello, World!";
        let chunks = chunker.chunk(data).unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].size, 8);
        assert_eq!(chunks[1].size, 7);
    }

    #[test]
    fn test_fast_chunker() {
        let mut chunker = FastChunker::new(4, 16, 8);
        let data = b"Hello, World! This is a test.";
        let chunks = chunker.chunk(data).unwrap();

        assert!(!chunks.is_empty());
        for chunk in chunks {
            assert!(chunk.size >= 4);
            assert!(chunk.size <= 16);
        }
    }

    #[test]
    fn test_calculate_power() {
        assert_eq!(calculate_power(2, 0), 1);
        assert_eq!(calculate_power(2, 3), 8);
        assert_eq!(calculate_power(3, 2), 9);
    }
}
