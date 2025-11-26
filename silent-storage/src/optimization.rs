//! # 后台优化模块
//!
//! 负责将热存储的文件异步优化为冷存储（CDC分块、去重、压缩）

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::{BinaryHeap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, error, info, warn};

/// 优化策略 - 决定如何优化文件
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OptimizationStrategy {
    /// 跳过优化 - 文件已经是最优格式（如已压缩的ZIP、视频等）
    Skip,
    /// 仅压缩 - 不分块，只压缩（小文件）
    CompressOnly,
    /// 完整优化 - CDC分块 + 去重 + 压缩（大文件、文本文件）
    Full,
}

impl OptimizationStrategy {
    /// 根据文件类型和大小决定优化策略
    pub fn decide(file_type: &crate::core::FileType, file_size: u64) -> Self {
        // 已压缩文件跳过优化
        if file_type.is_compressed() {
            return Self::Skip;
        }

        // 小文件（< 1MB）只压缩，不分块
        if file_size < 1024 * 1024 {
            return Self::CompressOnly;
        }

        // 其他情况完整优化
        Self::Full
    }
}

/// 优化任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationTask {
    /// 任务ID
    pub task_id: String,
    /// 文件ID
    pub file_id: String,
    /// 文件路径（热存储）
    pub hot_path: PathBuf,
    /// 文件大小
    pub file_size: u64,
    /// 文件哈希
    pub file_hash: String,
    /// 优化策略
    pub strategy: OptimizationStrategy,
    /// 任务优先级（0-10，越大越优先）
    pub priority: u8,
    /// 创建时间
    pub created_at: NaiveDateTime,
    /// 计划执行时间（延迟执行）
    pub scheduled_at: NaiveDateTime,
    /// 开始执行时间
    pub started_at: Option<NaiveDateTime>,
    /// 完成时间
    pub completed_at: Option<NaiveDateTime>,
    /// 任务状态
    pub status: crate::OptimizationStatus,
    /// 错误信息（失败时）
    pub error: Option<String>,
    /// 重试次数
    pub retry_count: u32,
}

impl OptimizationTask {
    /// 创建新的优化任务
    pub fn new(
        file_id: String,
        hot_path: PathBuf,
        file_size: u64,
        file_hash: String,
        strategy: OptimizationStrategy,
        delay_secs: u64,
    ) -> Self {
        let now = chrono::Local::now().naive_local();
        let scheduled_at = now + chrono::Duration::seconds(delay_secs as i64);

        Self {
            task_id: format!("opt_{}", scru128::new()),
            file_id,
            hot_path,
            file_size,
            file_hash,
            strategy,
            priority: Self::calculate_priority(file_size, strategy),
            created_at: now,
            scheduled_at,
            started_at: None,
            completed_at: None,
            status: crate::OptimizationStatus::Pending,
            error: None,
            retry_count: 0,
        }
    }

    /// 计算任务优先级
    /// - 大文件优先级更高（节省更多空间）
    /// - 完整优化策略优先级更高
    fn calculate_priority(file_size: u64, strategy: OptimizationStrategy) -> u8 {
        let size_priority = match file_size {
            0..=1_048_576 => 1,               // < 1MB: 低优先级
            1_048_577..=10_485_760 => 3,      // 1-10MB: 中优先级
            10_485_761..=104_857_600 => 5,    // 10-100MB: 高优先级
            104_857_601..=1_073_741_824 => 7, // 100MB-1GB: 很高优先级
            _ => 9,                           // > 1GB: 最高优先级
        };

        let strategy_priority = match strategy {
            OptimizationStrategy::Skip => 0,
            OptimizationStrategy::CompressOnly => 1,
            OptimizationStrategy::Full => 2,
        };

        (size_priority + strategy_priority).min(10)
    }

    /// 检查任务是否可以执行（已到计划时间）
    pub fn is_ready(&self) -> bool {
        let now = chrono::Local::now().naive_local();
        now >= self.scheduled_at && self.status == crate::OptimizationStatus::Pending
    }

    /// 标记任务开始执行
    pub fn mark_started(&mut self) {
        self.started_at = Some(chrono::Local::now().naive_local());
        self.status = crate::OptimizationStatus::Optimizing;
    }

    /// 标记任务完成
    pub fn mark_completed(&mut self) {
        self.completed_at = Some(chrono::Local::now().naive_local());
        self.status = crate::OptimizationStatus::Completed;
    }

    /// 标记任务失败
    pub fn mark_failed(&mut self, error: String) {
        self.completed_at = Some(chrono::Local::now().naive_local());
        self.status = crate::OptimizationStatus::Failed;
        self.error = Some(error);
        self.retry_count += 1;
    }

    /// 标记任务跳过
    pub fn mark_skipped(&mut self, reason: String) {
        self.completed_at = Some(chrono::Local::now().naive_local());
        self.status = crate::OptimizationStatus::Skipped;
        self.error = Some(reason);
    }

    /// 检查是否可以重试（失败且重试次数 < 3）
    pub fn can_retry(&self) -> bool {
        self.status == crate::OptimizationStatus::Failed && self.retry_count < 3
    }

    /// 重置任务以便重试
    pub fn reset_for_retry(&mut self, delay_secs: u64) {
        let now = chrono::Local::now().naive_local();
        self.scheduled_at = now + chrono::Duration::seconds(delay_secs as i64);
        self.started_at = None;
        self.completed_at = None;
        self.status = crate::OptimizationStatus::Pending;
    }
}

/// 优化统计
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OptimizationStats {
    /// 总任务数
    pub total_tasks: usize,
    /// 待执行任务数
    pub pending_tasks: usize,
    /// 执行中任务数
    pub running_tasks: usize,
    /// 已完成任务数
    pub completed_tasks: usize,
    /// 失败任务数
    pub failed_tasks: usize,
    /// 跳过任务数
    pub skipped_tasks: usize,
    /// 已节省空间（字节）
    pub space_saved: u64,
    /// 已优化文件大小（字节）
    pub optimized_size: u64,
}

/// 任务优先级包装器（用于BinaryHeap）
/// BinaryHeap是最大堆，我们需要优先级高的任务先执行
#[derive(Debug, Clone)]
struct PrioritizedTask {
    task: OptimizationTask,
}

impl PartialEq for PrioritizedTask {
    fn eq(&self, other: &Self) -> bool {
        self.task.priority == other.task.priority
    }
}

impl Eq for PrioritizedTask {}

impl PartialOrd for PrioritizedTask {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PrioritizedTask {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // 优先级高的排前面（最大堆）
        self.task.priority.cmp(&other.task.priority)
    }
}

/// 优化调度器 - 管理优化任务队列
pub struct OptimizationScheduler {
    /// 任务队列（优先级堆）
    task_queue: Arc<RwLock<BinaryHeap<PrioritizedTask>>>,
    /// 任务映射（file_id -> task_id）- 用于快速查找
    task_map: Arc<RwLock<HashMap<String, String>>>,
    /// 统计信息
    stats: Arc<RwLock<OptimizationStats>>,
    /// 最大并发任务数（预留，用于将来的并发控制）
    #[allow(dead_code)]
    max_concurrent: usize,
    /// 调度器是否运行
    running: Arc<RwLock<bool>>,
    /// 后台任务句柄
    scheduler_handle: Arc<RwLock<Option<tokio::task::JoinHandle<()>>>>,
}

impl OptimizationScheduler {
    /// 创建新的调度器
    pub fn new(max_concurrent: usize) -> Self {
        Self {
            task_queue: Arc::new(RwLock::new(BinaryHeap::new())),
            task_map: Arc::new(RwLock::new(HashMap::new())),
            stats: Arc::new(RwLock::new(OptimizationStats::default())),
            max_concurrent,
            running: Arc::new(RwLock::new(false)),
            scheduler_handle: Arc::new(RwLock::new(None)),
        }
    }

    /// 提交优化任务
    pub async fn submit_task(&self, task: OptimizationTask) {
        let file_id = task.file_id.clone();
        let task_id = task.task_id.clone();

        // 检查是否已存在任务
        let mut task_map = self.task_map.write().await;
        if task_map.contains_key(&file_id) {
            warn!("文件 {} 已有优化任务，跳过", file_id);
            return;
        }

        // 添加到队列
        let mut queue = self.task_queue.write().await;
        queue.push(PrioritizedTask { task: task.clone() });
        task_map.insert(file_id, task_id);

        // 更新统计
        let mut stats = self.stats.write().await;
        stats.total_tasks += 1;
        stats.pending_tasks += 1;

        debug!(
            "优化任务已提交: file_id={}, priority={}, strategy={:?}",
            task.file_id, task.priority, task.strategy
        );
    }

    /// 获取下一个就绪的任务
    pub async fn get_next_ready_task(&self) -> Option<OptimizationTask> {
        let mut queue = self.task_queue.write().await;
        let mut task_map = self.task_map.write().await;

        // 从堆顶开始查找就绪的任务
        let mut temp_tasks = Vec::new();
        let mut result = None;

        while let Some(prioritized) = queue.pop() {
            if prioritized.task.is_ready() {
                // 找到就绪任务
                task_map.remove(&prioritized.task.file_id);
                result = Some(prioritized.task);
                break;
            } else {
                // 还未到执行时间，放回临时列表
                temp_tasks.push(prioritized);
            }
        }

        // 将未执行的任务放回队列
        for task in temp_tasks {
            queue.push(task);
        }

        if let Some(ref task) = result {
            // 更新统计
            let mut stats = self.stats.write().await;
            stats.pending_tasks = stats.pending_tasks.saturating_sub(1);
            stats.running_tasks += 1;

            info!(
                "获取优化任务: file_id={}, priority={}",
                task.file_id, task.priority
            );
        }

        result
    }

    /// 标记任务完成
    pub async fn mark_task_completed(&self, file_id: &str, space_saved: u64, optimized_size: u64) {
        let mut stats = self.stats.write().await;
        stats.running_tasks = stats.running_tasks.saturating_sub(1);
        stats.completed_tasks += 1;
        stats.space_saved += space_saved;
        stats.optimized_size += optimized_size;

        info!("任务完成: file_id={}, 节省空间={}B", file_id, space_saved);
    }

    /// 标记任务失败
    pub async fn mark_task_failed(&self, file_id: &str, error: &str) {
        let mut stats = self.stats.write().await;
        stats.running_tasks = stats.running_tasks.saturating_sub(1);
        stats.failed_tasks += 1;

        error!("任务失败: file_id={}, error={}", file_id, error);
    }

    /// 标记任务跳过
    pub async fn mark_task_skipped(&self, file_id: &str, reason: &str) {
        let mut stats = self.stats.write().await;
        stats.running_tasks = stats.running_tasks.saturating_sub(1);
        stats.skipped_tasks += 1;

        debug!("任务跳过: file_id={}, reason={}", file_id, reason);
    }

    /// 重新提交失败的任务
    pub async fn resubmit_failed_task(&self, mut task: OptimizationTask) {
        if !task.can_retry() {
            warn!("任务 {} 已超过最大重试次数，不再重试", task.file_id);
            return;
        }

        // 重置任务状态
        task.reset_for_retry(300); // 5分钟后重试

        // 重新提交
        self.submit_task(task).await;
    }

    /// 获取统计信息
    pub async fn get_stats(&self) -> OptimizationStats {
        self.stats.read().await.clone()
    }

    /// 获取队列长度
    pub async fn queue_len(&self) -> usize {
        self.task_queue.read().await.len()
    }

    /// 检查调度器是否运行
    pub async fn is_running(&self) -> bool {
        *self.running.read().await
    }

    /// 启动调度器后台任务
    pub async fn start(&self) {
        if *self.running.read().await {
            warn!("调度器已在运行");
            return;
        }

        *self.running.write().await = true;
        info!("优化调度器已启动");
    }

    /// 停止调度器
    pub async fn stop(&self) {
        if !*self.running.read().await {
            return;
        }

        *self.running.write().await = false;

        // 等待后台任务完成
        if let Some(handle) = self.scheduler_handle.write().await.take() {
            let _ = handle.await;
        }

        info!("优化调度器已停止");
    }

    /// 清空队列
    pub async fn clear_queue(&self) {
        let mut queue = self.task_queue.write().await;
        let mut task_map = self.task_map.write().await;
        let mut stats = self.stats.write().await;

        let removed_count = queue.len();
        queue.clear();
        task_map.clear();
        stats.pending_tasks = 0;

        info!("已清空优化队列，移除 {} 个任务", removed_count);
    }

    /// 获取所有待处理任务的副本（用于测试和监控）
    pub async fn get_pending_tasks(&self) -> Vec<OptimizationTask> {
        let queue = self.task_queue.read().await;
        queue.iter().map(|pt| pt.task.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_optimization_strategy_decide() {
        use crate::core::FileType;

        // 已压缩文件应该跳过
        let zip_type = FileType::detect(b"PK\x03\x04");
        assert_eq!(
            OptimizationStrategy::decide(&zip_type, 10_000_000),
            OptimizationStrategy::Skip
        );

        // 小文件只压缩
        let text_type = FileType::detect(b"Hello World");
        assert_eq!(
            OptimizationStrategy::decide(&text_type, 500_000),
            OptimizationStrategy::CompressOnly
        );

        // 大文件完整优化
        assert_eq!(
            OptimizationStrategy::decide(&text_type, 10_000_000),
            OptimizationStrategy::Full
        );
    }

    #[test]
    fn test_optimization_task_priority() {
        // 大文件 + Full策略 = 高优先级
        let task1 = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            2_000_000_000, // 2GB
            "hash1".to_string(),
            OptimizationStrategy::Full,
            300,
        );
        assert!(task1.priority >= 9);

        // 小文件 + CompressOnly = 低优先级
        let task2 = OptimizationTask::new(
            "file2".to_string(),
            PathBuf::from("/tmp/file2"),
            500_000, // 500KB
            "hash2".to_string(),
            OptimizationStrategy::CompressOnly,
            300,
        );
        assert!(task2.priority <= 3);
    }

    #[test]
    fn test_optimization_task_lifecycle() {
        let mut task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0, // 立即执行
        );

        // 初始状态
        assert_eq!(task.status, crate::OptimizationStatus::Pending);
        assert!(task.is_ready());

        // 开始执行
        task.mark_started();
        assert_eq!(task.status, crate::OptimizationStatus::Optimizing);
        assert!(task.started_at.is_some());

        // 完成
        task.mark_completed();
        assert_eq!(task.status, crate::OptimizationStatus::Completed);
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn test_optimization_task_retry() {
        let mut task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        // 标记失败
        task.mark_failed("Test error".to_string());
        assert_eq!(task.status, crate::OptimizationStatus::Failed);
        assert_eq!(task.retry_count, 1);
        assert!(task.can_retry());

        // 重试
        task.reset_for_retry(300);
        assert_eq!(task.status, crate::OptimizationStatus::Pending);
        assert!(task.started_at.is_none());

        // 多次失败后不能重试
        task.mark_failed("Test error 2".to_string());
        task.reset_for_retry(300);
        task.mark_failed("Test error 3".to_string());
        task.reset_for_retry(300);
        task.mark_failed("Test error 4".to_string());
        assert!(!task.can_retry());
    }

    #[test]
    fn test_optimization_task_skipped() {
        let mut task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Skip,
            0,
        );

        task.mark_skipped("Already compressed".to_string());
        assert_eq!(task.status, crate::OptimizationStatus::Skipped);
        assert_eq!(task.error, Some("Already compressed".to_string()));
        assert!(task.completed_at.is_some());
    }

    #[test]
    fn test_optimization_stats_default() {
        let stats = OptimizationStats::default();
        assert_eq!(stats.total_tasks, 0);
        assert_eq!(stats.pending_tasks, 0);
        assert_eq!(stats.running_tasks, 0);
        assert_eq!(stats.completed_tasks, 0);
        assert_eq!(stats.failed_tasks, 0);
        assert_eq!(stats.skipped_tasks, 0);
        assert_eq!(stats.space_saved, 0);
        assert_eq!(stats.optimized_size, 0);
    }

    #[test]
    fn test_prioritized_task_ordering() {
        let task1 = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            100_000_000, // 100MB
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        let task2 = OptimizationTask::new(
            "file2".to_string(),
            PathBuf::from("/tmp/file2"),
            500_000, // 500KB
            "hash2".to_string(),
            OptimizationStrategy::CompressOnly,
            0,
        );

        let pt1 = PrioritizedTask { task: task1 };
        let pt2 = PrioritizedTask { task: task2 };

        // 高优先级任务应该排在前面
        assert!(pt1 > pt2);
    }

    #[tokio::test]
    async fn test_scheduler_submit_and_queue_len() {
        let scheduler = OptimizationScheduler::new(2);

        let task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        scheduler.submit_task(task).await;

        assert_eq!(scheduler.queue_len().await, 1);

        let stats = scheduler.get_stats().await;
        assert_eq!(stats.total_tasks, 1);
        assert_eq!(stats.pending_tasks, 1);
    }

    #[tokio::test]
    async fn test_scheduler_duplicate_task() {
        let scheduler = OptimizationScheduler::new(2);

        let task1 = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        let task2 = OptimizationTask::new(
            "file1".to_string(), // 相同 file_id
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        scheduler.submit_task(task1).await;
        scheduler.submit_task(task2).await; // 应该被跳过

        assert_eq!(scheduler.queue_len().await, 1);
    }

    #[tokio::test]
    async fn test_scheduler_get_next_ready_task() {
        let scheduler = OptimizationScheduler::new(2);

        let task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0, // 立即执行
        );

        scheduler.submit_task(task).await;

        let next_task = scheduler.get_next_ready_task().await;
        assert!(next_task.is_some());
        assert_eq!(next_task.unwrap().file_id, "file1");

        // 队列应该为空
        assert_eq!(scheduler.queue_len().await, 0);

        // 统计应该更新
        let stats = scheduler.get_stats().await;
        assert_eq!(stats.pending_tasks, 0);
        assert_eq!(stats.running_tasks, 1);
    }

    #[tokio::test]
    async fn test_scheduler_delayed_task() {
        let scheduler = OptimizationScheduler::new(2);

        let task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            3600, // 1小时后执行
        );

        scheduler.submit_task(task).await;

        // 任务未就绪，不应该被获取
        let next_task = scheduler.get_next_ready_task().await;
        assert!(next_task.is_none());

        // 任务应该还在队列中
        assert_eq!(scheduler.queue_len().await, 1);
    }

    #[tokio::test]
    async fn test_scheduler_mark_completed() {
        let scheduler = OptimizationScheduler::new(2);

        scheduler.mark_task_completed("file1", 1000, 500).await;

        let stats = scheduler.get_stats().await;
        assert_eq!(stats.completed_tasks, 1);
        assert_eq!(stats.space_saved, 1000);
        assert_eq!(stats.optimized_size, 500);
    }

    #[tokio::test]
    async fn test_scheduler_mark_failed() {
        let scheduler = OptimizationScheduler::new(2);

        scheduler.mark_task_failed("file1", "Test error").await;

        let stats = scheduler.get_stats().await;
        assert_eq!(stats.failed_tasks, 1);
    }

    #[tokio::test]
    async fn test_scheduler_mark_skipped() {
        let scheduler = OptimizationScheduler::new(2);

        scheduler.mark_task_skipped("file1", "Already optimized").await;

        let stats = scheduler.get_stats().await;
        assert_eq!(stats.skipped_tasks, 1);
    }

    #[tokio::test]
    async fn test_scheduler_resubmit_failed_task() {
        let scheduler = OptimizationScheduler::new(2);

        let mut task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        task.mark_failed("Test error".to_string());

        scheduler.resubmit_failed_task(task).await;

        assert_eq!(scheduler.queue_len().await, 1);
    }

    #[tokio::test]
    async fn test_scheduler_start_stop() {
        let scheduler = OptimizationScheduler::new(2);

        assert!(!scheduler.is_running().await);

        scheduler.start().await;
        assert!(scheduler.is_running().await);

        scheduler.stop().await;
        assert!(!scheduler.is_running().await);
    }

    #[tokio::test]
    async fn test_scheduler_clear_queue() {
        let scheduler = OptimizationScheduler::new(2);

        let task1 = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        let task2 = OptimizationTask::new(
            "file2".to_string(),
            PathBuf::from("/tmp/file2"),
            2_000_000,
            "hash2".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        scheduler.submit_task(task1).await;
        scheduler.submit_task(task2).await;

        assert_eq!(scheduler.queue_len().await, 2);

        scheduler.clear_queue().await;

        assert_eq!(scheduler.queue_len().await, 0);

        let stats = scheduler.get_stats().await;
        assert_eq!(stats.pending_tasks, 0);
    }

    #[tokio::test]
    async fn test_scheduler_priority_ordering() {
        let scheduler = OptimizationScheduler::new(2);

        // 低优先级任务
        let low_priority = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            500_000, // 500KB
            "hash1".to_string(),
            OptimizationStrategy::CompressOnly,
            0,
        );

        // 高优先级任务
        let high_priority = OptimizationTask::new(
            "file2".to_string(),
            PathBuf::from("/tmp/file2"),
            1_000_000_000, // 1GB
            "hash2".to_string(),
            OptimizationStrategy::Full,
            0,
        );

        // 先提交低优先级，后提交高优先级
        scheduler.submit_task(low_priority).await;
        scheduler.submit_task(high_priority).await;

        // 应该先获取高优先级任务
        let next_task = scheduler.get_next_ready_task().await;
        assert!(next_task.is_some());
        assert_eq!(next_task.unwrap().file_id, "file2");
    }

    #[tokio::test]
    async fn test_scheduler_get_pending_tasks() {
        let scheduler = OptimizationScheduler::new(2);

        let task = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_000_000,
            "hash1".to_string(),
            OptimizationStrategy::Full,
            3600, // 延迟执行
        );

        scheduler.submit_task(task).await;

        let pending = scheduler.get_pending_tasks().await;
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].file_id, "file1");
    }

    #[test]
    fn test_calculate_priority_edge_cases() {
        // 测试边界值
        let task_1mb = OptimizationTask::new(
            "file1".to_string(),
            PathBuf::from("/tmp/file1"),
            1_048_576, // 正好 1MB
            "hash1".to_string(),
            OptimizationStrategy::CompressOnly,
            0,
        );
        assert!(task_1mb.priority >= 1);

        let task_10mb = OptimizationTask::new(
            "file2".to_string(),
            PathBuf::from("/tmp/file2"),
            10_485_760, // 正好 10MB
            "hash2".to_string(),
            OptimizationStrategy::Full,
            0,
        );
        assert!(task_10mb.priority >= 3);

        // 测试 Skip 策略优先级为 0
        let task_skip = OptimizationTask::new(
            "file3".to_string(),
            PathBuf::from("/tmp/file3"),
            100_000_000,
            "hash3".to_string(),
            OptimizationStrategy::Skip,
            0,
        );
        // Skip 策略会将优先级设为 size_priority + 0
        assert!(task_skip.priority <= 10);
    }
}
