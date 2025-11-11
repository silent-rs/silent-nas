//! 冷热数据分离模块
//!
//! 实现基于LRU的访问频率统计和冷热数据自动分层存储

use crate::error::{StorageError, Result};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// 存储层级
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StorageTier {
    /// 热数据（SSD，频繁访问）
    Hot,
    /// 温数据（普通硬盘，偶尔访问）
    Warm,
    /// 冷数据（归档存储，长期不访问）
    Cold,
}

impl StorageTier {
    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            StorageTier::Hot => "hot",
            StorageTier::Warm => "warm",
            StorageTier::Cold => "cold",
        }
    }
}

impl FromStr for StorageTier {
    type Err = ();
    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s {
            "hot" => Ok(StorageTier::Hot),
            "warm" => Ok(StorageTier::Warm),
            "cold" => Ok(StorageTier::Cold),
            _ => Err(()),
        }
    }
}

/// 层级配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierConfig {
    /// 热数据容量（字节）
    pub hot_capacity: u64,
    /// 温数据容量（字节）
    pub warm_capacity: u64,
    /// 冷数据容量（字节，0表示无限制）
    pub cold_capacity: u64,
    /// 自动迁移间隔（秒）
    pub migration_interval_secs: u64,
    /// LRU窗口大小
    pub lru_window_size: usize,
    /// 热数据阈值（最近N次访问）
    pub hot_access_threshold: u32,
    /// 温数据阈值（最近N次访问）
    pub warm_access_threshold: u32,
}

impl Default for TierConfig {
    fn default() -> Self {
        Self {
            hot_capacity: 10 * 1024 * 1024 * 1024,   // 10GB
            warm_capacity: 100 * 1024 * 1024 * 1024, // 100GB
            cold_capacity: 0,                        // 无限制
            migration_interval_secs: 3600,           // 1小时
            lru_window_size: 10000,
            hot_access_threshold: 100,
            warm_access_threshold: 10,
        }
    }
}

/// 访问记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessRecord {
    /// 文件ID
    pub file_id: String,
    /// 访问时间
    pub accessed_at: chrono::NaiveDateTime,
    /// 访问次数（窗口内）
    pub access_count: u32,
}

/// 数据项信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataItem {
    /// 文件ID
    pub file_id: String,
    /// 文件大小
    pub size: u64,
    /// 当前层级
    pub tier: StorageTier,
    /// 存储路径
    pub storage_path: PathBuf,
    /// 创建时间
    pub created_at: chrono::NaiveDateTime,
    /// 最后访问时间
    pub last_accessed: chrono::NaiveDateTime,
    /// 总访问次数
    pub total_accesses: u32,
    /// 是否已压缩
    pub is_compressed: bool,
}

/// 分层存储管理器
pub struct TieredStorage {
    config: TierConfig,
    /// 层级根目录
    tier_roots: HashMap<StorageTier, PathBuf>,
    /// 内存中的数据项映射
    items: RwLock<HashMap<String, DataItem>>,
    /// LRU访问队列
    lru_queue: RwLock<VecDeque<String>>,
    /// 层级使用统计
    #[allow(dead_code)]
    tier_usage: RwLock<HashMap<StorageTier, u64>>,
    /// 当前层级使用量
    tier_sizes: RwLock<HashMap<StorageTier, u64>>,
}

impl TieredStorage {
    pub fn new(config: TierConfig, base_path: &str) -> Self {
        let base = Path::new(base_path);
        let tier_roots = HashMap::from([
            (StorageTier::Hot, base.join("hot")),
            (StorageTier::Warm, base.join("warm")),
            (StorageTier::Cold, base.join("cold")),
        ]);

        Self {
            config,
            tier_roots,
            items: RwLock::new(HashMap::new()),
            lru_queue: RwLock::new(VecDeque::new()),
            tier_usage: RwLock::new(HashMap::new()),
            tier_sizes: RwLock::new(HashMap::new()),
        }
    }

    /// 初始化分层存储
    pub async fn init(&self) -> Result<()> {
        // 创建各层级目录
        for (tier, path) in &self.tier_roots {
            tokio::fs::create_dir_all(path)
                .await
                .map_err(StorageError::Io)?;
            info!("初始化存储层级 {:?}: {:?}", tier.as_str(), path);
        }

        // 加载现有数据项
        self.load_existing_items().await?;

        info!("分层存储初始化完成");
        Ok(())
    }

    /// 记录数据访问
    pub async fn record_access(&self, file_id: &str) -> Result<()> {
        let mut items = self.items.write().await;
        let mut lru_queue = self.lru_queue.write().await;

        if let Some(item) = items.get_mut(file_id) {
            // 更新访问信息
            item.last_accessed = chrono::Local::now().naive_local();
            item.total_accesses += 1;

            // 更新LRU队列
            lru_queue.retain(|id| id != file_id);
            lru_queue.push_back(file_id.to_string());

            // 保持LRU窗口大小
            while lru_queue.len() > self.config.lru_window_size {
                lru_queue.pop_front();
            }
        }

        Ok(())
    }

    /// 获取数据项信息
    pub async fn get_item(&self, file_id: &str) -> Option<DataItem> {
        let items = self.items.read().await;
        items.get(file_id).cloned()
    }

    /// 将数据项分配到合适的层级
    pub async fn assign_tier(
        &self,
        file_id: &str,
        size: u64,
        storage_path: PathBuf,
    ) -> Result<StorageTier> {
        // 计算推荐层级（在获取写锁之前）
        let recommended_tier = self.calculate_recommended_tier(file_id).await;

        // 检查容量限制（在获取写锁之前）
        let tier = if self.can_fit_in_tier(recommended_tier, size).await {
            recommended_tier
        } else {
            // 容量不足，选择下一个层级
            self.find_available_tier(size)
                .await
                .unwrap_or(recommended_tier)
        };

        // 现在获取写锁进行实际操作
        let mut items = self.items.write().await;

        // 提取文件名
        let file_name = storage_path.file_name().unwrap_or_default();
        let file_name_path = std::path::Path::new(file_name).to_path_buf();

        // 创建数据项
        let item = DataItem {
            file_id: file_id.to_string(),
            size,
            tier,
            storage_path: storage_path.clone(),
            created_at: chrono::Local::now().naive_local(),
            last_accessed: chrono::Local::now().naive_local(),
            total_accesses: 0,
            is_compressed: false,
        };

        // 检查目标层级
        let _target_path = self
            .tier_roots
            .get(&tier)
            .ok_or_else(|| StorageError::Storage(format!("未找到层级 {:?} 的根目录", tier.as_str())))?
            .join(file_name_path);

        // 实际移动文件由调用者处理
        items.insert(file_id.to_string(), item);
        drop(items); // 显式释放 items 写锁

        // 更新使用统计
        let mut tier_sizes = self.tier_sizes.write().await;
        *tier_sizes.entry(tier).or_insert(0) += size;

        info!("数据项 {} 分配到层级 {:?}", file_id, tier.as_str());
        Ok(tier)
    }

    /// 计算推荐的存储层级
    async fn calculate_recommended_tier(&self, file_id: &str) -> StorageTier {
        let lru_queue = self.lru_queue.read().await;
        let items = self.items.read().await;

        // 获取访问计数
        let access_count = if let Some(item) = items.get(file_id) {
            item.total_accesses
        } else {
            0
        };

        // 计算在LRU队列中的位置
        let lru_position = lru_queue.iter().position(|id| id == file_id);
        let recency_score = if let Some(pos) = lru_position {
            (lru_queue.len() - pos) as u32
        } else {
            0
        };

        // 综合评分
        let score = access_count + recency_score;

        if score >= self.config.hot_access_threshold {
            StorageTier::Hot
        } else if score >= self.config.warm_access_threshold {
            StorageTier::Warm
        } else {
            StorageTier::Cold
        }
    }

    /// 检查是否可以在指定层级中容纳
    async fn can_fit_in_tier(&self, tier: StorageTier, size: u64) -> bool {
        let tier_sizes = self.tier_sizes.read().await;
        let current_size = tier_sizes.get(&tier).copied().unwrap_or(0);
        let capacity = match tier {
            StorageTier::Hot => self.config.hot_capacity,
            StorageTier::Warm => self.config.warm_capacity,
            StorageTier::Cold => self.config.cold_capacity,
        };

        if capacity == 0 {
            return true; // 无限制
        }

        current_size + size <= capacity
    }

    /// 查找可用的存储层级
    async fn find_available_tier(&self, size: u64) -> Option<StorageTier> {
        // 按优先级尝试各层级
        for tier in [StorageTier::Hot, StorageTier::Warm, StorageTier::Cold] {
            if self.can_fit_in_tier(tier, size).await {
                return Some(tier);
            }
        }
        None
    }

    /// 执行层级迁移
    pub async fn perform_migration(&self) -> Result<MigrationResult> {
        info!("开始执行层级迁移");

        let mut migrated = MigrationResult::default();

        // 按层级重新评估所有数据项
        let items = self.items.read().await;
        let mut items_to_migrate: Vec<(String, StorageTier)> = Vec::new();

        for (file_id, item) in items.iter() {
            let recommended_tier = self.calculate_recommended_tier(file_id).await;
            if recommended_tier != item.tier {
                items_to_migrate.push((file_id.clone(), recommended_tier));
            }
        }
        drop(items);

        // 执行迁移
        for (file_id, new_tier) in items_to_migrate {
            if let Some(result) = self.migrate_item(&file_id, new_tier).await? {
                migrated.migrated_count += 1;
                migrated.total_migrated_size += result.size;
            }
        }

        info!(
            "层级迁移完成: {} 项，{} 字节",
            migrated.migrated_count, migrated.total_migrated_size
        );
        Ok(migrated)
    }

    /// 迁移单个数据项
    async fn migrate_item(
        &self,
        file_id: &str,
        new_tier: StorageTier,
    ) -> Result<Option<MigrationItemResult>> {
        let mut items = self.items.write().await;

        if let Some(item) = items.get_mut(file_id) {
            let old_tier = item.tier;

            // 检查新层级容量
            if !self.can_fit_in_tier(new_tier, item.size).await {
                warn!(
                    "数据项 {} 无法迁移到层级 {:?}：容量不足",
                    file_id,
                    new_tier.as_str()
                );
                return Ok(None);
            }

            // 实际迁移操作（移动文件）
            // 这里需要调用存储管理器来实际移动文件
            // let result = self.storage_manager.move_file(item.storage_path, &new_tier_path).await?;

            // 更新数据项信息
            item.tier = new_tier;

            // 更新使用统计
            let mut tier_sizes = self.tier_sizes.write().await;
            *tier_sizes.entry(old_tier).or_insert(0) =
                tier_sizes.get(&old_tier).copied().unwrap_or(0) - item.size;
            *tier_sizes.entry(new_tier).or_insert(0) += item.size;

            let result = MigrationItemResult {
                file_id: file_id.to_string(),
                from_tier: old_tier,
                to_tier: new_tier,
                size: item.size,
            };

            info!(
                "数据项 {} 从层级 {:?} 迁移到 {:?}",
                file_id,
                old_tier.as_str(),
                new_tier.as_str()
            );
            return Ok(Some(result));
        }

        Ok(None)
    }

    /// 获取分层统计信息
    pub async fn get_tier_stats(&self) -> TierStats {
        let items = self.items.read().await;
        let mut stats = TierStats::new();

        for item in items.values() {
            match item.tier {
                StorageTier::Hot => {
                    stats.hot_count += 1;
                    stats.hot_size += item.size;
                }
                StorageTier::Warm => {
                    stats.warm_count += 1;
                    stats.warm_size += item.size;
                }
                StorageTier::Cold => {
                    stats.cold_count += 1;
                    stats.cold_size += item.size;
                }
            }
        }

        let tier_sizes = self.tier_sizes.read().await;
        stats.hot_capacity = self.config.hot_capacity;
        stats.warm_capacity = self.config.warm_capacity;
        stats.cold_capacity = self.config.cold_capacity;
        stats.hot_used = tier_sizes.get(&StorageTier::Hot).copied().unwrap_or(0);
        stats.warm_used = tier_sizes.get(&StorageTier::Warm).copied().unwrap_or(0);
        stats.cold_used = tier_sizes.get(&StorageTier::Cold).copied().unwrap_or(0);

        stats
    }

    /// 清理未引用的数据项
    pub async fn cleanup_unreferenced(&self) -> Result<u32> {
        let items = self.items.read().await;
        let lru_queue = self.lru_queue.read().await;

        // 查找LRU队列中不存在的数据项
        let mut to_remove = Vec::new();
        for file_id in items.keys() {
            if !lru_queue.contains(file_id) {
                to_remove.push(file_id.clone());
            }
        }

        // 标记为可删除（实际删除由外部处理）
        let count = to_remove.len();
        for file_id in to_remove {
            if let Some(item) = items.get(&file_id) {
                warn!(
                    "发现未引用的数据项: {} (层级: {:?})",
                    file_id,
                    item.tier.as_str()
                );
            }
        }

        Ok(count as u32)
    }

    /// 加载现有数据项
    async fn load_existing_items(&self) -> Result<()> {
        // 扫描各层级目录，加载现有的数据项
        for (tier, path) in &self.tier_roots {
            let mut entries = tokio::fs::read_dir(path).await.map_err(StorageError::Io)?;

            while let Some(entry) = entries.next_entry().await.map_err(StorageError::Io)? {
                let path = entry.path();
                if path.is_file()
                    && let Some(file_id) = path.file_name().and_then(|s| s.to_str())
                {
                    let metadata = entry.metadata().await.map_err(StorageError::Io)?;

                    let item = DataItem {
                        file_id: file_id.to_string(),
                        size: metadata.len(),
                        tier: *tier,
                        storage_path: path.clone(),
                        created_at: chrono::Local::now().naive_local(),
                        last_accessed: chrono::Local::now().naive_local(),
                        total_accesses: 0,
                        is_compressed: false,
                    };

                    self.items.write().await.insert(file_id.to_string(), item);
                }
            }
        }

        info!("加载了 {} 个数据项", self.items.read().await.len());
        Ok(())
    }
}

/// 迁移结果
#[derive(Debug, Default)]
pub struct MigrationResult {
    pub migrated_count: u32,
    pub total_migrated_size: u64,
}

/// 迁移项结果
#[derive(Debug, Clone)]
pub struct MigrationItemResult {
    pub file_id: String,
    pub from_tier: StorageTier,
    pub to_tier: StorageTier,
    pub size: u64,
}

/// 分层统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierStats {
    pub hot_count: u32,
    pub hot_size: u64,
    pub hot_capacity: u64,
    pub hot_used: u64,
    pub warm_count: u32,
    pub warm_size: u64,
    pub warm_capacity: u64,
    pub warm_used: u64,
    pub cold_count: u32,
    pub cold_size: u64,
    pub cold_capacity: u64,
    pub cold_used: u64,
}

impl Default for TierStats {
    fn default() -> Self {
        Self::new()
    }
}

impl TierStats {
    pub fn new() -> Self {
        Self {
            hot_count: 0,
            hot_size: 0,
            hot_capacity: 0,
            hot_used: 0,
            warm_count: 0,
            warm_size: 0,
            warm_capacity: 0,
            warm_used: 0,
            cold_count: 0,
            cold_size: 0,
            cold_capacity: 0,
            cold_used: 0,
        }
    }

    /// 获取热层级使用率
    pub fn hot_usage_rate(&self) -> f32 {
        if self.hot_capacity > 0 {
            self.hot_used as f32 / self.hot_capacity as f32
        } else {
            0.0
        }
    }

    /// 获取温层级使用率
    pub fn warm_usage_rate(&self) -> f32 {
        if self.warm_capacity > 0 {
            self.warm_used as f32 / self.warm_capacity as f32
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_storage_tier_from_str() {
        assert_eq!(StorageTier::from_str("hot"), Ok(StorageTier::Hot));
        assert_eq!(StorageTier::from_str("warm"), Ok(StorageTier::Warm));
        assert_eq!(StorageTier::from_str("cold"), Ok(StorageTier::Cold));
        assert_eq!(StorageTier::from_str("invalid"), Err(()));
    }

    #[test]
    fn test_storage_tier_as_str() {
        assert_eq!(StorageTier::Hot.as_str(), "hot");
        assert_eq!(StorageTier::Warm.as_str(), "warm");
        assert_eq!(StorageTier::Cold.as_str(), "cold");
    }

    #[tokio::test]
    async fn test_tiered_storage_new() {
        let temp_dir = TempDir::new().unwrap();
        let config = TierConfig::default();
        let storage = TieredStorage::new(config, temp_dir.path().to_str().unwrap());

        storage.init().await.unwrap();

        let stats = storage.get_tier_stats().await;
        assert_eq!(stats.hot_count, 0);
        assert_eq!(stats.warm_count, 0);
        assert_eq!(stats.cold_count, 0);
    }

    #[tokio::test]
    async fn test_record_access() {
        let temp_dir = TempDir::new().unwrap();
        let config = TierConfig::default();
        let storage = TieredStorage::new(config, temp_dir.path().to_str().unwrap());
        storage.init().await.unwrap();

        let file_id = "test_file";
        let size = 1024;
        let storage_path = temp_dir.path().to_path_buf().join("test_file");

        storage
            .assign_tier(file_id, size, storage_path)
            .await
            .unwrap();
        storage.record_access(file_id).await.unwrap();

        let item = storage.get_item(file_id).await.unwrap();
        assert_eq!(item.total_accesses, 1);
    }
}
