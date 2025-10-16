use crate::error::Result;
use crate::models::FileEvent;
use crate::storage::StorageManager;
use crate::sync::{FileSync, SyncManager};
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
}

impl EventListener {
    /// 创建事件监听器
    pub fn new(
        sync_manager: Arc<SyncManager>,
        nats_client: async_nats::Client,
        topic_prefix: String,
        storage: StorageManager,
    ) -> Self {
        Self {
            sync_manager,
            nats_client,
            topic_prefix,
            storage: Arc::new(storage),
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
                                let url = format!(
                                    "{}/api/files/{}",
                                    source_http.trim_end_matches('/'),
                                    event.file_id
                                );
                                match reqwest::get(&url).await {
                                    Ok(resp) if resp.status().is_success() => {
                                        match resp.bytes().await {
                                            Ok(bytes) => {
                                                if let Err(e) = self
                                                    .storage
                                                    .save_file(&event.file_id, &bytes)
                                                    .await
                                                {
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
                                                "{}/webdav{}",
                                                source_http.trim_end_matches('/'),
                                                dav_path
                                            );
                                            match reqwest::get(&dav_url).await {
                                                Ok(r2) if r2.status().is_success() => {
                                                    if let Ok(bytes) = r2.bytes().await {
                                                        if let Err(e) = self
                                                            .storage
                                                            .save_file(&event.file_id, &bytes)
                                                            .await
                                                        {
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
