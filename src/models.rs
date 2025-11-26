// 重新导出 silent-nas-core 中的核心类型
pub use silent_nas_core::{EventType, FileEvent, FileMetadata, FileVersion};

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_metadata() -> FileMetadata {
        FileMetadata {
            id: "test_id_123".to_string(),
            name: "test_file.txt".to_string(),
            path: "/test/test_file.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: chrono::Local::now().naive_local(),
            modified_at: chrono::Local::now().naive_local(),
        }
    }

    #[test]
    fn test_file_metadata_creation() {
        let metadata = create_test_metadata();

        assert_eq!(metadata.id, "test_id_123");
        assert_eq!(metadata.name, "test_file.txt");
        assert_eq!(metadata.path, "/test/test_file.txt");
        assert_eq!(metadata.size, 1024);
        assert_eq!(metadata.hash, "abc123");
    }

    #[test]
    fn test_file_metadata_serialization() {
        let metadata = create_test_metadata();

        // 序列化
        let json = serde_json::to_string(&metadata).unwrap();
        assert!(json.contains("test_id_123"));
        assert!(json.contains("test_file.txt"));

        // 反序列化
        let deserialized: FileMetadata = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, metadata.id);
        assert_eq!(deserialized.name, metadata.name);
    }

    #[test]
    fn test_file_metadata_clone() {
        let metadata = create_test_metadata();
        let cloned = metadata.clone();

        assert_eq!(metadata.id, cloned.id);
        assert_eq!(metadata.name, cloned.name);
    }

    #[test]
    fn test_event_type_created() {
        let event_type = EventType::Created;
        let json = serde_json::to_string(&event_type).unwrap();
        assert_eq!(json, "\"created\"");
    }

    #[test]
    fn test_event_type_modified() {
        let event_type = EventType::Modified;
        let json = serde_json::to_string(&event_type).unwrap();
        assert_eq!(json, "\"modified\"");
    }

    #[test]
    fn test_event_type_deleted() {
        let event_type = EventType::Deleted;
        let json = serde_json::to_string(&event_type).unwrap();
        assert_eq!(json, "\"deleted\"");
    }

    #[test]
    fn test_event_type_deserialization() {
        let created: EventType = serde_json::from_str("\"created\"").unwrap();
        let modified: EventType = serde_json::from_str("\"modified\"").unwrap();
        let deleted: EventType = serde_json::from_str("\"deleted\"").unwrap();

        matches!(created, EventType::Created);
        matches!(modified, EventType::Modified);
        matches!(deleted, EventType::Deleted);
    }

    #[test]
    fn test_file_event_new_created() {
        let metadata = create_test_metadata();
        let event = FileEvent::new(
            EventType::Created,
            "file123".to_string(),
            Some(metadata.clone()),
        );

        assert!(!event.event_id.is_empty());
        assert_eq!(event.file_id, "file123");
        assert!(event.metadata.is_some());
        matches!(event.event_type, EventType::Created);
    }

    #[test]
    fn test_file_event_new_deleted() {
        let event = FileEvent::new(EventType::Deleted, "file456".to_string(), None);

        assert!(!event.event_id.is_empty());
        assert_eq!(event.file_id, "file456");
        assert!(event.metadata.is_none());
        matches!(event.event_type, EventType::Deleted);
    }

    #[test]
    fn test_file_event_serialization() {
        let metadata = create_test_metadata();
        let event = FileEvent::new(EventType::Modified, "file789".to_string(), Some(metadata));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("file789"));
        assert!(json.contains("modified"));

        let deserialized: FileEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.file_id, "file789");
    }

    #[test]
    fn test_file_event_with_metadata() {
        let metadata = create_test_metadata();
        let event = FileEvent::new(
            EventType::Created,
            "file_id".to_string(),
            Some(metadata.clone()),
        );

        assert!(event.metadata.is_some());
        let event_metadata = event.metadata.unwrap();
        assert_eq!(event_metadata.id, "test_id_123");
        assert_eq!(event_metadata.name, "test_file.txt");
    }

    #[test]
    fn test_file_event_without_metadata() {
        let event = FileEvent::new(EventType::Deleted, "file_id".to_string(), None);
        assert!(event.metadata.is_none());
    }

    #[test]
    fn test_file_event_unique_ids() {
        let event1 = FileEvent::new(EventType::Created, "file1".to_string(), None);
        let event2 = FileEvent::new(EventType::Created, "file1".to_string(), None);

        // 事件ID应该不同
        assert_ne!(event1.event_id, event2.event_id);
    }

    #[test]
    fn test_file_event_clone() {
        let metadata = create_test_metadata();
        let event = FileEvent::new(EventType::Modified, "file_id".to_string(), Some(metadata));
        let cloned = event.clone();

        assert_eq!(event.event_id, cloned.event_id);
        assert_eq!(event.file_id, cloned.file_id);
    }
}
