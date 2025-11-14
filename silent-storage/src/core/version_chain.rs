//! 版本链深度管理模块
//!
//! 该模块提供版本链深度检测和自动合并功能，
//! 避免版本链过长导致恢复性能退化。

use crate::error::{Result, StorageError};
use crate::{ChunkInfo, FileDelta, VersionInfo};
use std::collections::HashMap;

/// 版本链深度配置
#[derive(Debug, Clone)]
pub struct VersionChainConfig {
    /// 最大版本链深度（超过此深度触发合并）
    pub max_depth: usize,
    /// 合并后保留的版本数（最近N个版本）
    pub keep_recent: usize,
}

impl Default for VersionChainConfig {
    fn default() -> Self {
        Self {
            max_depth: 5,   // 默认最大5层
            keep_recent: 2, // 合并后保留最近2个版本
        }
    }
}

/// 版本链信息
#[derive(Debug, Clone)]
pub struct VersionChain {
    /// 版本链（从新到旧）
    pub versions: Vec<VersionInfo>,
    /// 当前深度
    pub depth: usize,
    /// 是否需要合并
    pub needs_merge: bool,
}

/// 版本链管理器
pub struct VersionChainManager {
    config: VersionChainConfig,
}

impl VersionChainManager {
    pub fn new(config: VersionChainConfig) -> Self {
        Self { config }
    }

    /// 构建版本链（从当前版本回溯）
    ///
    /// # 参数
    /// - version_loader: 版本加载函数，根据 version_id 返回 VersionInfo
    pub fn build_chain<F>(
        &self,
        current_version: &VersionInfo,
        version_loader: F,
    ) -> Result<VersionChain>
    where
        F: Fn(&str) -> Result<Option<VersionInfo>>,
    {
        let mut versions = vec![current_version.clone()];
        let mut current = current_version.clone();
        let mut depth = 1;

        // 回溯父版本
        while let Some(ref parent_id) = current.parent_version_id {
            if let Some(parent) = version_loader(parent_id)? {
                versions.push(parent.clone());
                current = parent;
                depth += 1;

                // 防止无限循环（检测环）
                if depth > 100 {
                    return Err(StorageError::Storage(
                        "版本链深度超过100层，可能存在循环引用".to_string(),
                    ));
                }
            } else {
                break;
            }
        }

        let needs_merge = depth > self.config.max_depth;

        Ok(VersionChain {
            versions,
            depth,
            needs_merge,
        })
    }

    /// 判断是否需要合并版本链
    pub fn should_merge(&self, chain: &VersionChain) -> bool {
        chain.needs_merge
    }

    /// 计算需要合并的版本数
    pub fn calculate_merge_count(&self, chain: &VersionChain) -> usize {
        chain.depth.saturating_sub(self.config.keep_recent)
    }

    /// 生成合并计划
    ///
    /// 返回需要保留的版本和需要合并的版本
    pub fn generate_merge_plan(&self, chain: &VersionChain) -> MergePlan {
        let merge_count = self.calculate_merge_count(chain);

        if merge_count == 0 {
            return MergePlan {
                keep_versions: chain.versions.clone(),
                merge_versions: vec![],
                new_base_version_id: chain.versions.last().map(|v| v.version_id.clone()),
            };
        }

        // 保留最近的版本
        let keep_versions = chain.versions[..self.config.keep_recent].to_vec();

        // 需要合并的版本（从 keep_recent 到末尾）
        let merge_versions = chain.versions[self.config.keep_recent..].to_vec();

        // 新的基础版本ID（最早的保留版本）
        let new_base_version_id = keep_versions.last().map(|v| v.version_id.clone());

        MergePlan {
            keep_versions,
            merge_versions,
            new_base_version_id,
        }
    }

    /// 合并版本链中的块数据
    ///
    /// 将多个增量版本合并为一个完整版本
    pub fn merge_chunks(
        &self,
        plan: &MergePlan,
        delta_loader: impl Fn(&str) -> Result<Option<FileDelta>>,
    ) -> Result<MergedVersion> {
        if plan.merge_versions.is_empty() {
            return Err(StorageError::Storage("没有需要合并的版本".to_string()));
        }

        // 从最老的版本开始，逐步应用差异
        let mut chunk_map: HashMap<String, ChunkInfo> = HashMap::new();

        // 按时间顺序（从旧到新）遍历需要合并的版本
        for version in plan.merge_versions.iter().rev() {
            if let Some(delta) = delta_loader(&version.version_id)? {
                for chunk in delta.chunks {
                    // 使用最新的块覆盖旧的块（基于 offset）
                    let key = format!("{}-{}", chunk.offset, chunk.size);
                    chunk_map.insert(key, chunk);
                }
            }
        }

        // 按 offset 排序
        let mut merged_chunks: Vec<ChunkInfo> = chunk_map.into_values().collect();
        merged_chunks.sort_by_key(|c| c.offset);

        // 计算合并后的大小
        let chunk_count = merged_chunks.len();
        let total_size: u64 = merged_chunks.iter().map(|c| c.size as u64).sum();
        let storage_size: u64 = chunk_count as u64 * 8192; // 假设平均 8KB/块

        Ok(MergedVersion {
            chunks: merged_chunks,
            file_size: total_size,
            storage_size,
            chunk_count,
        })
    }
}

/// 合并计划
#[derive(Debug, Clone)]
pub struct MergePlan {
    /// 保留的版本（最近的N个）
    pub keep_versions: Vec<VersionInfo>,
    /// 需要合并的版本
    pub merge_versions: Vec<VersionInfo>,
    /// 新的基础版本ID（最早的保留版本）
    pub new_base_version_id: Option<String>,
}

/// 合并后的版本
#[derive(Debug, Clone)]
pub struct MergedVersion {
    /// 合并后的块列表
    pub chunks: Vec<ChunkInfo>,
    /// 文件大小
    pub file_size: u64,
    /// 存储大小
    pub storage_size: u64,
    /// 块数量
    pub chunk_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    fn create_test_version(version_id: &str, parent_id: Option<&str>) -> VersionInfo {
        VersionInfo {
            version_id: version_id.to_string(),
            file_id: "test_file".to_string(),
            parent_version_id: parent_id.map(|s| s.to_string()),
            file_size: 1000,
            chunk_count: 10,
            storage_size: 500,
            created_at: Local::now().naive_local(),
            is_current: version_id == "v5",
        }
    }

    #[test]
    fn test_build_chain() {
        let manager = VersionChainManager::new(VersionChainConfig::default());

        // 创建5层版本链: v5 -> v4 -> v3 -> v2 -> v1
        let v5 = create_test_version("v5", Some("v4"));
        let v4 = create_test_version("v4", Some("v3"));
        let v3 = create_test_version("v3", Some("v2"));
        let v2 = create_test_version("v2", Some("v1"));
        let v1 = create_test_version("v1", None);

        let versions = [v1, v2, v3, v4, v5.clone()];

        let loader = |version_id: &str| {
            Ok(versions
                .iter()
                .find(|v| v.version_id == version_id)
                .cloned())
        };

        let chain = manager.build_chain(&v5, loader).unwrap();

        assert_eq!(chain.depth, 5);
        assert_eq!(chain.versions.len(), 5);
        assert_eq!(chain.versions[0].version_id, "v5");
        assert_eq!(chain.versions[4].version_id, "v1");
    }

    #[test]
    fn test_should_merge() {
        let manager = VersionChainManager::new(VersionChainConfig {
            max_depth: 5,
            keep_recent: 2,
        });

        // 深度5，不需要合并（刚好等于max_depth）
        let chain_5 = VersionChain {
            versions: vec![
                create_test_version("v5", Some("v4")),
                create_test_version("v4", Some("v3")),
                create_test_version("v3", Some("v2")),
                create_test_version("v2", Some("v1")),
                create_test_version("v1", None),
            ],
            depth: 5,
            needs_merge: false,
        };
        assert!(!manager.should_merge(&chain_5));

        // 深度6，需要合并
        let chain_6 = VersionChain {
            versions: vec![
                create_test_version("v6", Some("v5")),
                create_test_version("v5", Some("v4")),
                create_test_version("v4", Some("v3")),
                create_test_version("v3", Some("v2")),
                create_test_version("v2", Some("v1")),
                create_test_version("v1", None),
            ],
            depth: 6,
            needs_merge: true,
        };
        assert!(manager.should_merge(&chain_6));
    }

    #[test]
    fn test_generate_merge_plan() {
        let manager = VersionChainManager::new(VersionChainConfig {
            max_depth: 5,
            keep_recent: 2,
        });

        let chain = VersionChain {
            versions: vec![
                create_test_version("v6", Some("v5")),
                create_test_version("v5", Some("v4")),
                create_test_version("v4", Some("v3")),
                create_test_version("v3", Some("v2")),
                create_test_version("v2", Some("v1")),
                create_test_version("v1", None),
            ],
            depth: 6,
            needs_merge: true,
        };

        let plan = manager.generate_merge_plan(&chain);

        // 保留最近2个版本 (v6, v5)
        assert_eq!(plan.keep_versions.len(), 2);
        assert_eq!(plan.keep_versions[0].version_id, "v6");
        assert_eq!(plan.keep_versions[1].version_id, "v5");

        // 合并4个版本 (v4, v3, v2, v1)
        assert_eq!(plan.merge_versions.len(), 4);
        assert_eq!(plan.merge_versions[0].version_id, "v4");
        assert_eq!(plan.merge_versions[3].version_id, "v1");

        // 新基础版本为v5
        assert_eq!(plan.new_base_version_id.as_deref(), Some("v5"));
    }

    #[test]
    fn test_calculate_merge_count() {
        let manager = VersionChainManager::new(VersionChainConfig {
            max_depth: 5,
            keep_recent: 2,
        });

        let chain_6 = VersionChain {
            versions: vec![],
            depth: 6,
            needs_merge: true,
        };

        assert_eq!(manager.calculate_merge_count(&chain_6), 4); // 6 - 2 = 4
    }
}
