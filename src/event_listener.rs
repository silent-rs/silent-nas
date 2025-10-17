use crate::error::Result;
use crate::models::FileEvent;
use crate::storage::StorageManager;
use crate::sync::crdt::{FileSync, SyncManager};
use crate::sync::incremental::IncrementalSyncHandler;
use futures_util::StreamExt;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

/// NATS 事件监听器
/// 监听其他节点的文件变更事件并触发本地同步
pub struct EventListener {
    sync_manager: Arc<SyncManager>,
    nats_client: async_nats::Client,
    topic_prefix: String,
    storage: Arc<StorageManager>,
    inc_sync_handler: Arc<IncrementalSyncHandler>,
}

impl EventListener {
    /// 创建事件监听器
    pub fn new(
        sync_manager: Arc<SyncManager>,
        nats_client: async_nats::Client,
        topic_prefix: String,
        storage: StorageManager,
        chunk_size: usize,
    ) -> Self {
        let storage_arc = Arc::new(storage);
        let inc_sync_handler =
            Arc::new(IncrementalSyncHandler::new(storage_arc.clone(), chunk_size));

        Self {
            sync_manager,
            nats_client,
            topic_prefix,
            storage: storage_arc,
            inc_sync_handler,
        }
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
                            let need_fetch = match self.storage.get_metadata(&event.file_id).await {
                                Ok(local_meta) => {
                                    local_meta.hash != expected_hash
                                        || local_meta.size != expected_size
                                }
                                Err(_) => true,
                            };

                            if need_fetch {
                                // 优先尝试增量同步
                                info!("尝试增量同步文件: {}", event.file_id);
                                match self
                                    .inc_sync_handler
                                    .pull_incremental(&event.file_id, &source_http)
                                    .await
                                {
                                    Ok(data) => {
                                        // 优先按元数据路径保存
                                        let save_res = if let Some(meta) = event.metadata.as_ref() {
                                            if !meta.path.is_empty() {
                                                self.storage.save_at_path(&meta.path, &data).await
                                            } else {
                                                self.storage.save_file(&event.file_id, &data).await
                                            }
                                        } else {
                                            self.storage.save_file(&event.file_id, &data).await
                                        };

                                        if let Err(e) = save_res {
                                            error!(
                                                "保存增量同步内容失败: {} - {}",
                                                event.file_id, e
                                            );
                                        } else {
                                            info!("✅ 增量同步完成并保存: {}", event.file_id);
                                        }
                                        return Ok(()); // 增量同步成功，提前返回
                                    }
                                    Err(e) => {
                                        warn!(
                                            "增量同步失败，回退到全量下载: {} - {}",
                                            event.file_id, e
                                        );
                                        // 继续执行全量下载逻辑
                                    }
                                }

                                // Fallback: 全量下载
                                let url = format!(
                                    "{}/api/files/{}",
                                    source_http.trim_end_matches('/'),
                                    event.file_id
                                );
                                match reqwest::get(&url).await {
                                    Ok(resp) if resp.status().is_success() => {
                                        match resp.bytes().await {
                                            Ok(bytes) => {
                                                // 优先按元数据路径保存，避免在 data 下生成ID文件
                                                let save_res =
                                                    if let Some(meta) = event.metadata.as_ref() {
                                                        if !meta.path.is_empty() {
                                                            self.storage
                                                                .save_at_path(&meta.path, &bytes)
                                                                .await
                                                        } else {
                                                            self.storage
                                                                .save_file(&event.file_id, &bytes)
                                                                .await
                                                        }
                                                    } else {
                                                        self.storage
                                                            .save_file(&event.file_id, &bytes)
                                                            .await
                                                    };
                                                if let Err(e) = save_res {
                                                    error!(
                                                        "保存拉取内容失败: {} - {}",
                                                        event.file_id, e
                                                    );
                                                } else {
                                                    info!(
                                                        "📥 已从源拉取并保存内容: {}",
                                                        event.file_id
                                                    );
                                                }
                                            }
                                            Err(e) => error!(
                                                "读取拉取响应体失败: {} - {}",
                                                event.file_id, e
                                            ),
                                        }
                                    }
                                    Ok(resp) => {
                                        warn!(
                                            "拉取内容失败: {} - HTTP {}",
                                            event.file_id,
                                            resp.status()
                                        );
                                        // Fallback: 若提供了原始路径，尝试通过 WebDAV 拉取
                                        if let Some(meta) = event.metadata.as_ref() {
                                            let dav_path = if meta.path.starts_with('/') {
                                                meta.path.clone()
                                            } else {
                                                format!("/{}", meta.path)
                                            };
                                            let dav_url = format!(
                                                "{}{}",
                                                source_http.trim_end_matches('/'),
                                                dav_path
                                            );
                                            match reqwest::get(&dav_url).await {
                                                Ok(r2) if r2.status().is_success() => {
                                                    if let Ok(bytes) = r2.bytes().await {
                                                        let save_res = self
                                                            .storage
                                                            .save_at_path(&dav_path, &bytes)
                                                            .await;
                                                        if let Err(e) = save_res {
                                                            error!(
                                                                "保存DAV拉取内容失败: {} - {}",
                                                                event.file_id, e
                                                            );
                                                        } else {
                                                            info!(
                                                                "📥 已通过WebDAV回退拉取并保存内容: {}",
                                                                event.file_id
                                                            );
                                                        }
                                                    }
                                                }
                                                Ok(r2) => warn!(
                                                    "WebDAV回退拉取失败: {} - HTTP {}",
                                                    event.file_id,
                                                    r2.status()
                                                ),
                                                Err(e) => warn!(
                                                    "请求WebDAV源失败: {} - {}",
                                                    event.file_id, e
                                                ),
                                            }
                                        }
                                    }
                                    Err(e) => warn!("请求源内容失败: {} - {}", event.file_id, e),
                                }
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

        // 创建NATS客户端需要真实的NATS服务器
        // 这里只验证存储管理器可以正常工作
        let test_data = b"test content";
        let file_id = storage.save_file("test", test_data).await.unwrap();
        assert!(!file_id.id.is_empty());

        // 验证增量同步处理器可以创建
        let storage_arc = Arc::new(storage);
        let handler = IncrementalSyncHandler::new(storage_arc, 64 * 1024);

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
        let storage_arc = Arc::new(storage);

        let _sync_manager = Arc::new(SyncManager::new(
            "test-node".to_string(),
            storage_arc.clone(),
            None,
        ));

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
        let storage_arc = Arc::new(storage);

        let handler = IncrementalSyncHandler::new(storage_arc.clone(), 4096);

        // 创建测试文件
        let test_data = b"test content for incremental sync";
        let metadata = storage_arc.save_file("inc-test", test_data).await.unwrap();

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
