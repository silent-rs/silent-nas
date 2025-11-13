use crate::error::Result;
use crate::models::FileEvent;
use crate::sync::crdt::{FileSync, SyncManager};
use crate::sync::incremental::IncrementalSyncHandler;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use silent_nas_core::StorageManagerTrait;
use std::sync::Arc;
use tokio::time::{Duration, sleep};
use tracing::{debug, error, info, warn};

fn jittered_secs(base: u64) -> u64 {
    let jitter = rand::random::<f64>() * 0.4 + 0.8; // 0.8~1.2
    ((base as f64) * jitter).round() as u64
}

/// NATS 事件监听器
/// 监听其他节点的文件变更事件并触发本地同步
pub struct EventListener {
    sync_manager: Arc<SyncManager>,
    nats_client: async_nats::Client,
    topic_prefix: String,
    inc_sync_handler: Arc<IncrementalSyncHandler>,
    // 拉取/退避配置
    http_connect_timeout: u64,
    http_request_timeout: u64,
    fetch_max_retries: u32,
    fetch_base_backoff: u64,
    fetch_max_backoff: u64,
}

impl EventListener {
    /// 创建事件监听器
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sync_manager: Arc<SyncManager>,
        nats_client: async_nats::Client,
        topic_prefix: String,
        chunk_size: usize,
        http_connect_timeout: u64,
        http_request_timeout: u64,
        fetch_max_retries: u32,
        fetch_base_backoff: u64,
        fetch_max_backoff: u64,
    ) -> Self {
        let inc_sync_handler = Arc::new(IncrementalSyncHandler::new(chunk_size));

        Self {
            sync_manager,
            nats_client,
            topic_prefix,
            inc_sync_handler,
            http_connect_timeout,
            http_request_timeout,
            fetch_max_retries,
            fetch_base_backoff,
            fetch_max_backoff,
        }
    }

    fn backoff_delay(&self, attempt: u32) -> Duration {
        let factor = 1u64 << attempt.min(6);
        let mut secs = self.fetch_base_backoff.saturating_mul(factor);
        if secs > self.fetch_max_backoff {
            secs = self.fetch_max_backoff;
        }
        Duration::from_secs(jittered_secs(secs).max(1))
    }

    /// 启动事件监听
    pub async fn start(self) -> Result<()> {
        let node_id = self.sync_manager.node_id().to_string();
        info!("启动事件监听器: node_id={}", node_id);

        // 订阅所有文件事件（使用通配符）
        let topic_pattern = format!("{}.*", self.topic_prefix);
        let mut subscriber = self
            .nats_client
            .subscribe(topic_pattern.clone())
            .await
            .map_err(|e| crate::error::NasError::Nats(format!("订阅主题失败: {}", e)))?;

        info!("开始监听主题: {}", topic_pattern);

        // 持续监听消息
        while let Some(message) = subscriber.next().await {
            if let Err(e) = self.handle_event(&message.payload).await {
                error!("处理事件失败: {}", e);
            }
        }

        warn!("事件监听器已停止");
        Ok(())
    }

    /// 处理接收到的事件
    async fn handle_event(&self, payload: &[u8]) -> Result<()> {
        // 解析事件
        let event: FileEvent = serde_json::from_slice(payload)
            .map_err(|e| crate::error::NasError::Storage(format!("解析事件失败: {}", e)))?;

        debug!(
            "收到远程事件: file_id={}, event_type={:?}",
            event.file_id, event.event_type
        );

        // 处理事件
        match event.metadata.as_ref() {
            Some(metadata) => {
                let expected_size = metadata.size;
                let expected_hash = metadata.hash.clone();
                // 从元数据创建 FileSync 状态
                let file_sync = FileSync::new(
                    event.file_id.clone(),
                    metadata.clone(),
                    self.sync_manager.node_id(),
                );

                // 调用同步管理器处理远程同步
                match self.sync_manager.handle_remote_sync(file_sync).await {
                    Ok(_) => {
                        info!("✅ 成功处理远程文件同步: {}", event.file_id);

                        // 尝试内容拉取：若提供了源HTTP地址且本地不存在或哈希不一致
                        if let Some(source_http) = event.source_http_addr.clone() {
                            // 记录源地址
                            self.sync_manager
                                .set_last_source(&event.file_id, &source_http)
                                .await;
                            let need_fetch = match crate::storage::storage()
                                .get_metadata(&event.file_id)
                                .await
                            {
                                Ok(local_meta) => {
                                    local_meta.hash != expected_hash
                                        || local_meta.size != expected_size
                                }
                                Err(_) => true,
                            };

                            if need_fetch {
                                // 优先尝试增量同步（完成后进行端到端哈希校验）
                                info!("尝试增量同步文件: {}", event.file_id);
                                match self
                                    .inc_sync_handler
                                    .pull_incremental(&event.file_id, &source_http)
                                    .await
                                {
                                    Ok(data) => {
                                        let actual = format!("{:x}", Sha256::digest(&data));
                                        if actual == expected_hash {
                                            let save_res =
                                                if let Some(meta) = event.metadata.as_ref() {
                                                    if !meta.path.is_empty() {
                                                        crate::storage::storage()
                                                            .save_at_path(&meta.path, &data)
                                                            .await
                                                    } else {
                                                        crate::storage::storage()
                                                            .save_file(&event.file_id, &data)
                                                            .await
                                                    }
                                                } else {
                                                    crate::storage::storage()
                                                        .save_file(&event.file_id, &data)
                                                        .await
                                                };
                                            match save_res {
                                                Ok(_) => {
                                                    crate::metrics::record_sync_operation(
                                                        "incremental",
                                                        "success",
                                                        data.len() as u64,
                                                    );
                                                    info!(
                                                        "✅ 增量同步完成并通过哈希校验: {}",
                                                        event.file_id
                                                    );
                                                    return Ok(());
                                                }
                                                Err(e) => {
                                                    error!(
                                                        "保存增量同步内容失败: {} - {}",
                                                        event.file_id, e
                                                    );
                                                }
                                            }
                                        } else {
                                            warn!(
                                                "增量同步哈希不一致: {} expected={} actual={}",
                                                event.file_id, expected_hash, actual
                                            );
                                        }
                                    }
                                    Err(e) => {
                                        warn!(
                                            "增量同步失败，回退到全量下载: {} - {}",
                                            event.file_id, e
                                        );
                                    }
                                }

                                // 回退：全量下载（API/WebDAV）带重试、退避与哈希校验
                                let client = reqwest::Client::builder()
                                    .connect_timeout(Duration::from_secs(self.http_connect_timeout))
                                    .timeout(Duration::from_secs(self.http_request_timeout))
                                    .build()
                                    .unwrap_or_else(|_| reqwest::Client::new());

                                let api_url = format!(
                                    "{}/api/files/{}",
                                    source_http.trim_end_matches('/'),
                                    event.file_id
                                );

                                let mut last_err: Option<String> = None;
                                for attempt in 0..=self.fetch_max_retries {
                                    match client.get(&api_url).send().await {
                                        Ok(resp) if resp.status().is_success() => {
                                            match resp.bytes().await {
                                                Ok(bytes) => {
                                                    let actual =
                                                        format!("{:x}", Sha256::digest(&bytes));
                                                    if actual != expected_hash {
                                                        last_err = Some(format!(
                                                            "哈希不一致 expected={} actual={}",
                                                            expected_hash, actual
                                                        ));
                                                    } else {
                                                        let save_res = if let Some(meta) =
                                                            event.metadata.as_ref()
                                                        {
                                                            if !meta.path.is_empty() {
                                                                crate::storage::storage()
                                                                    .save_at_path(
                                                                        &meta.path, &bytes,
                                                                    )
                                                                    .await
                                                            } else {
                                                                crate::storage::storage()
                                                                    .save_file(
                                                                        &event.file_id,
                                                                        &bytes,
                                                                    )
                                                                    .await
                                                            }
                                                        } else {
                                                            crate::storage::storage()
                                                                .save_file(&event.file_id, &bytes)
                                                                .await
                                                        };
                                                        if let Err(e) = save_res {
                                                            last_err =
                                                                Some(format!("保存失败: {}", e));
                                                        } else {
                                                            crate::metrics::record_sync_operation(
                                                                "full",
                                                                "success",
                                                                bytes.len() as u64,
                                                            );
                                                            info!(
                                                                "📥 全量拉取并保存成功: {}",
                                                                event.file_id
                                                            );
                                                            return Ok(());
                                                        }
                                                    }
                                                }
                                                Err(e) => {
                                                    last_err = Some(format!("读取响应失败: {}", e));
                                                }
                                            }
                                        }
                                        Ok(resp) => {
                                            last_err = Some(format!("HTTP {}", resp.status()));
                                        }
                                        Err(e) => {
                                            last_err = Some(format!("请求失败: {}", e));
                                        }
                                    }
                                    if attempt < self.fetch_max_retries {
                                        let d = self.backoff_delay(attempt);
                                        debug!(
                                            "拉取重试: {} 尝试={} 等待={:?}",
                                            event.file_id,
                                            attempt + 1,
                                            d
                                        );
                                        sleep(d).await;
                                        continue;
                                    }
                                }
                                crate::metrics::record_sync_operation("full", "error", 0);
                                warn!(
                                    "全量拉取失败: {} - {}",
                                    event.file_id,
                                    last_err.unwrap_or_else(|| "unknown".into())
                                );
                            } else {
                                debug!("本地与远端一致，跳过内容拉取: {}", event.file_id);
                            }
                        } else {
                            debug!(
                                "事件未携带 source_http_addr，跳过内容拉取: {}",
                                event.file_id
                            );
                        }
                    }
                    Err(e) => {
                        error!("❌ 处理远程同步失败: {} - {}", event.file_id, e);
                    }
                }
            }
            None => {
                warn!("事件缺少元数据: file_id={}", event.file_id);
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageManager;
    use std::path::PathBuf;
    use tempfile::TempDir;

    // 注意：由于EventListener依赖NATS客户端，完整的功能测试需要集成测试环境
    // 这里只测试可以独立测试的部分

    #[tokio::test]
    async fn test_event_listener_dependencies() {
        // 测试EventListener的依赖项可以正确创建
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(PathBuf::from(temp_dir.path()), 64 * 1024);
        storage.init().await.unwrap();

        // 初始化全局storage
        let _ = crate::storage::init_global_storage(storage.clone());

        // 创建NATS客户端需要真实的NATS服务器
        // 这里只验证存储管理器可以正常工作
        let test_data = b"test content";
        let file_id = storage.save_file("test", test_data).await.unwrap();
        assert!(!file_id.id.is_empty());

        // 验证增量同步处理器可以创建
        let handler = IncrementalSyncHandler::new(64 * 1024);

        // 验证处理器可以正常工作
        let sig = handler
            .calculate_local_signature(&file_id.id)
            .await
            .unwrap();
        assert_eq!(sig.file_size, test_data.len() as u64);
    }

    #[test]
    fn test_module_imports() {
        // 验证所有必要的类型都可以正确导入
        // 这是一个编译时测试，如果编译通过就说明导入正确
        let _result: Result<()> = Ok(());
        // 测试通过意味着所有类型导入都正确
    }

    #[tokio::test]
    async fn test_event_listener_creation() {
        // 测试可以创建EventListener（不启动）
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(PathBuf::from(temp_dir.path()), 64 * 1024);

        // 初始化全局storage
        let _ = crate::storage::init_global_storage(storage.clone());

        let _sync_manager = Arc::new(SyncManager::new("test-node".to_string(), None));

        // 创建模拟的NATS客户端需要真实服务器，这里只是验证类型
        // 实际的EventListener创建需要集成测试环境
    }

    #[test]
    fn test_file_event_parsing() {
        use crate::models::{EventType, FileEvent, FileMetadata};

        // 测试事件序列化和反序列化
        let metadata = FileMetadata {
            id: "test-id".to_string(),
            name: "test.txt".to_string(),
            path: "/test/path".to_string(),
            size: 1024,
            hash: "testhash".to_string(),
            created_at: chrono::Local::now().naive_local(),
            modified_at: chrono::Local::now().naive_local(),
        };

        let event = FileEvent::new(
            EventType::Created,
            "test-id".to_string(),
            Some(metadata.clone()),
        );

        // 序列化
        let json = serde_json::to_string(&event).unwrap();
        assert!(!json.is_empty());

        // 反序列化
        let parsed: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.file_id, "test-id");
        assert_eq!(parsed.event_type, EventType::Created);
        assert!(parsed.metadata.is_some());
    }

    #[tokio::test]
    async fn test_storage_operations() {
        // 测试存储操作的基本功能
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(PathBuf::from(temp_dir.path()), 64 * 1024);
        storage.init().await.unwrap();

        // 测试保存文件
        let test_data = b"test content for event listener";
        let metadata = storage.save_file("event-test", test_data).await.unwrap();
        assert_eq!(metadata.size, test_data.len() as u64);

        // 测试读取文件
        let read_data = storage.read_file(&metadata.id).await.unwrap();
        assert_eq!(read_data, test_data);

        // 测试获取元数据
        let meta = storage.get_metadata(&metadata.id).await.unwrap();
        assert_eq!(meta.size, test_data.len() as u64);
        assert!(!meta.hash.is_empty());
    }

    #[tokio::test]
    async fn test_incremental_sync_handler() {
        // 测试增量同步处理器的基本功能
        let temp_dir = TempDir::new().unwrap();
        let storage = StorageManager::new(PathBuf::from(temp_dir.path()), 64 * 1024);
        storage.init().await.unwrap();

        // 初始化全局storage
        let _ = crate::storage::init_global_storage(storage.clone());

        let handler = IncrementalSyncHandler::new(4096);

        // 创建测试文件
        let test_data = b"test content for incremental sync";
        let metadata = storage.save_file("inc-test", test_data).await.unwrap();

        // 计算签名
        let signature = handler
            .calculate_local_signature(&metadata.id)
            .await
            .unwrap();

        assert_eq!(signature.file_size, test_data.len() as u64);
        assert!(!signature.chunks.is_empty());
        assert_eq!(signature.chunk_size, 4096);
    }

    #[test]
    fn test_topic_pattern_format() {
        // 测试主题模式格式
        let topic_prefix = "silent.nas.files";
        let pattern = format!("{}.*", topic_prefix);
        assert_eq!(pattern, "silent.nas.files.*");

        // 测试不同的前缀
        let custom_prefix = "custom.prefix";
        let custom_pattern = format!("{}.*", custom_prefix);
        assert_eq!(custom_pattern, "custom.prefix.*");
    }

    #[test]
    fn test_url_formatting() {
        // 测试URL格式化逻辑
        let source_http = "http://example.com:8080";
        let file_id = "test-file-id";

        // API URL
        let api_url = format!(
            "{}/api/files/{}",
            source_http.trim_end_matches('/'),
            file_id
        );
        assert_eq!(api_url, "http://example.com:8080/api/files/test-file-id");

        // 带尾部斜杠的情况
        let source_with_slash = "http://example.com:8080/";
        let api_url2 = format!(
            "{}/api/files/{}",
            source_with_slash.trim_end_matches('/'),
            file_id
        );
        assert_eq!(api_url2, "http://example.com:8080/api/files/test-file-id");
    }

    #[test]
    fn test_path_formatting() {
        // 测试路径格式化逻辑
        let path_without_slash = "test/path";
        let formatted1 = if path_without_slash.starts_with('/') {
            path_without_slash.to_string()
        } else {
            format!("/{}", path_without_slash)
        };
        assert_eq!(formatted1, "/test/path");

        let path_with_slash = "/test/path";
        let formatted2 = if path_with_slash.starts_with('/') {
            path_with_slash.to_string()
        } else {
            format!("/{}", path_with_slash)
        };
        assert_eq!(formatted2, "/test/path");
    }
}
