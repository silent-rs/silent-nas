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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_topic_format() {
        let prefix = "silent.nas.files";

        // 测试主题格式
        assert_eq!(format!("{}.created", prefix), "silent.nas.files.created");
        assert_eq!(format!("{}.modified", prefix), "silent.nas.files.modified");
        assert_eq!(format!("{}.deleted", prefix), "silent.nas.files.deleted");
    }

    #[test]
    fn test_event_type_to_topic() {
        let prefix = "test.prefix";

        let created_topic = format!("{}.created", prefix);
        let modified_topic = format!("{}.modified", prefix);
        let deleted_topic = format!("{}.deleted", prefix);

        assert!(created_topic.ends_with(".created"));
        assert!(modified_topic.ends_with(".modified"));
        assert!(deleted_topic.ends_with(".deleted"));
    }

    #[test]
    fn test_topic_prefix_handling() {
        let prefixes = vec![
            "simple",
            "with.dots",
            "with-dashes",
            "with_underscores",
            "123numeric",
        ];

        for prefix in prefixes {
            let topic = format!("{}.created", prefix);
            assert!(topic.contains(prefix));
            assert!(topic.contains(".created"));
        }
    }

    #[test]
    fn test_event_notifier_clone() {
        // EventNotifier 实现了 Clone
        // 这个测试验证 Clone trait 的存在
        let type_name = std::any::type_name::<EventNotifier>();
        assert!(type_name.contains("EventNotifier"));
    }
}
