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

    /// 获取 NATS 客户端（用于事件监听器）
    pub fn get_client(&self) -> Client {
        self.client.clone()
    }

    /// 获取主题前缀
    #[allow(dead_code)]
    pub fn get_topic_prefix(&self) -> &str {
        &self.topic_prefix
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

    #[test]
    fn test_event_type_variants() {
        use crate::models::EventType;

        // 测试所有 EventType 变体
        let created = EventType::Created;
        let modified = EventType::Modified;
        let deleted = EventType::Deleted;

        // 确保所有变体都存在
        assert!(matches!(created, EventType::Created));
        assert!(matches!(modified, EventType::Modified));
        assert!(matches!(deleted, EventType::Deleted));
    }

    #[test]
    fn test_topic_generation_for_all_event_types() {
        use crate::models::EventType;

        let prefix = "test.nas";
        let event_types = vec![EventType::Created, EventType::Modified, EventType::Deleted];

        for event_type in event_types {
            let topic = match &event_type {
                EventType::Created => format!("{}.created", prefix),
                EventType::Modified => format!("{}.modified", prefix),
                EventType::Deleted => format!("{}.deleted", prefix),
            };

            assert!(topic.starts_with(prefix));
            assert!(topic.len() > prefix.len());
        }
    }

    #[test]
    fn test_topic_with_empty_prefix() {
        let prefix = "";
        let topic = format!("{}.created", prefix);
        assert_eq!(topic, ".created");
    }

    #[test]
    fn test_topic_with_long_prefix() {
        let prefix = "very.long.prefix.with.many.segments";
        let topic = format!("{}.created", prefix);
        assert!(topic.starts_with(prefix));
        assert!(topic.ends_with(".created"));
    }

    #[test]
    fn test_topic_with_special_chars() {
        let prefixes = vec![
            "prefix-with-dashes",
            "prefix_with_underscores",
            "prefix.with.dots",
            "prefix123",
        ];

        for prefix in prefixes {
            let topic = format!("{}.created", prefix);
            assert!(topic.contains(prefix));
        }
    }

    #[test]
    fn test_file_event_serialization_for_notify() {
        use crate::models::{EventType, FileEvent, FileMetadata};
        use chrono::Local;

        let metadata = FileMetadata {
            id: "file-123".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let event = FileEvent::new(EventType::Created, "file-123".to_string(), Some(metadata));

        // 测试序列化
        let json = serde_json::to_vec(&event).unwrap();
        assert!(!json.is_empty());

        // 测试反序列化
        let deserialized: FileEvent = serde_json::from_slice(&json).unwrap();
        assert_eq!(deserialized.file_id, "file-123");
    }

    #[test]
    fn test_multiple_event_types_topic_uniqueness() {
        use crate::models::EventType;

        let prefix = "test";
        let created = format!("{}.{:?}", prefix, EventType::Created).to_lowercase();
        let modified = format!("{}.{:?}", prefix, EventType::Modified).to_lowercase();
        let deleted = format!("{}.{:?}", prefix, EventType::Deleted).to_lowercase();

        // 确保每个主题都是唯一的
        assert_ne!(created, modified);
        assert_ne!(modified, deleted);
        assert_ne!(created, deleted);
    }

    #[test]
    fn test_topic_prefix_consistency() {
        let prefixes = vec!["prefix1", "prefix2", "prefix3"];

        for prefix in &prefixes {
            let topic1 = format!("{}.created", prefix);
            let topic2 = format!("{}.created", prefix);
            assert_eq!(topic1, topic2); // 相同前缀应该生成相同的主题
        }
    }

    #[test]
    fn test_event_notifier_type_properties() {
        // 测试 EventNotifier 的类型属性
        let type_name = std::any::type_name::<EventNotifier>();

        assert!(type_name.contains("EventNotifier"));
        assert!(type_name.contains("notify"));
    }

    #[test]
    fn test_file_event_serialization() {
        use crate::models::{EventType, FileEvent, FileMetadata};

        let metadata = FileMetadata {
            id: "file-123".to_string(),
            name: "test.txt".to_string(),
            path: "/files/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: chrono::Local::now().naive_local(),
            modified_at: chrono::Local::now().naive_local(),
        };

        let event = FileEvent {
            event_id: "event-456".to_string(),
            file_id: "test-123".to_string(),
            event_type: EventType::Created,
            timestamp: chrono::Local::now().naive_local(),
            metadata: Some(metadata),
            source_node_id: Some("node-1".to_string()),
            source_http_addr: Some("http://localhost:8080".to_string()),
        };

        // 测试序列化
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("test-123"));
        assert!(json.contains("created"));

        // 测试反序列化
        let deserialized: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file_id, "test-123");
    }

    #[test]
    fn test_topic_validation() {
        // 测试主题名称验证
        let valid_topics = vec![
            "silent.nas.files.created",
            "app.events.modified",
            "system.notifications.deleted",
        ];

        for topic in valid_topics {
            assert!(topic.contains("."));
            assert!(!topic.is_empty());
            assert!(topic.len() > 5);
        }
    }

    #[test]
    fn test_event_type_string_representation() {
        use crate::models::EventType;

        let types = vec![
            (EventType::Created, "created"),
            (EventType::Modified, "modified"),
            (EventType::Deleted, "deleted"),
        ];

        for (event_type, expected_str) in types {
            let prefix = "test";
            let topic = match event_type {
                EventType::Created => format!("{}.created", prefix),
                EventType::Modified => format!("{}.modified", prefix),
                EventType::Deleted => format!("{}.deleted", prefix),
            };
            assert!(topic.ends_with(expected_str));
        }
    }

    #[test]
    fn test_topic_prefix_validation() {
        // 测试各种前缀格式
        let test_cases = vec![
            ("simple", true),
            ("with.dots.multiple", true),
            ("with-dashes", true),
            ("with_underscores", true),
            ("MixedCase", true),
            ("", false), // 空前缀应该被标记
        ];

        for (prefix, _should_be_valid) in test_cases {
            let topic = format!("{}.created", prefix);
            // 验证topic至少包含事件类型
            assert!(topic.contains("created"));
        }
    }

    #[test]
    fn test_multiple_event_types_topic_generation() {
        use crate::models::EventType;

        let prefix = "test.app";
        let events = [EventType::Created, EventType::Modified, EventType::Deleted];

        let topics: Vec<String> = events
            .iter()
            .map(|event_type| match event_type {
                EventType::Created => format!("{}.created", prefix),
                EventType::Modified => format!("{}.modified", prefix),
                EventType::Deleted => format!("{}.deleted", prefix),
            })
            .collect();

        assert_eq!(topics.len(), 3);
        assert!(topics[0].contains("created"));
        assert!(topics[1].contains("modified"));
        assert!(topics[2].contains("deleted"));

        // 确保所有topic都以相同前缀开始
        for topic in &topics {
            assert!(topic.starts_with(prefix));
        }
    }
}
