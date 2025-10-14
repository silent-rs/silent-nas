use crate::error::{NasError, Result};
use crate::models::{EventType, FileEvent};
use async_nats::Client;
use tracing::{debug, error, info};

/// NATS 事件通知器
#[derive(Clone)]
pub struct EventNotifier {
    client: Client,
    topic_prefix: String,
}

impl EventNotifier {
    /// 连接到 NATS 服务器
    pub async fn connect(url: &str, topic_prefix: String) -> Result<Self> {
        let client = async_nats::connect(url)
            .await
            .map_err(|e| NasError::Nats(format!("连接 NATS 失败: {}", e)))?;
        info!("NATS 客户端已连接: {}", url);

        Ok(Self {
            client,
            topic_prefix,
        })
    }

    /// 获取主题名称
    fn get_topic(&self, event_type: &EventType) -> String {
        match event_type {
            EventType::Created => format!("{}.created", self.topic_prefix),
            EventType::Modified => format!("{}.modified", self.topic_prefix),
            EventType::Deleted => format!("{}.deleted", self.topic_prefix),
        }
    }

    /// 发布文件事件
    pub async fn publish_event(&self, event: &FileEvent) -> Result<()> {
        let topic = self.get_topic(&event.event_type);
        let payload = serde_json::to_vec(event)?;

        self.client
            .publish(topic.clone(), payload.into())
            .await
            .map_err(|e| NasError::Nats(format!("发布事件失败: {}", e)))?;

        debug!(
            "事件已发布: {} - 文件ID: {} - 事件ID: {}",
            topic, event.file_id, event.event_id
        );

        Ok(())
    }

    /// 发布文件创建事件
    pub async fn notify_created(&self, event: FileEvent) -> Result<()> {
        self.publish_event(&event).await
    }

    /// 发布文件修改事件
    #[allow(dead_code)]
    pub async fn notify_modified(&self, event: FileEvent) -> Result<()> {
        self.publish_event(&event).await
    }

    /// 发布文件删除事件
    pub async fn notify_deleted(&self, event: FileEvent) -> Result<()> {
        self.publish_event(&event).await
    }

    /// 批量发布事件
    #[allow(dead_code)]
    pub async fn publish_batch(&self, events: Vec<FileEvent>) -> Result<()> {
        for event in events {
            if let Err(e) = self.publish_event(&event).await {
                error!("发布事件失败: {} - {}", event.event_id, e);
            }
        }
        Ok(())
    }
}
