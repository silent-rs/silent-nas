//! WebDAV 大文件上传集成测试
//!
//! 测试上传会话管理、内存监控、秒传等功能的集成

#[cfg(test)]
mod tests {
    use super::super::instant_upload::InstantUploadManager;
    use super::super::memory_monitor::{MemoryGuard, MemoryMonitor};
    use super::super::upload_session::{UploadSession, UploadSessionManager, UploadStatus};
    use std::sync::Arc;

    #[tokio::test]
    async fn test_full_upload_workflow() {
        // 模拟完整的上传流程
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_1");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);
        let memory_monitor = MemoryMonitor::new(100, 80);
        let instant_upload = InstantUploadManager::new();

        let file_path = "/test/large_file.bin".to_string();
        let file_size = 10 * 1024 * 1024; // 10MB
        let chunk_size = 8 * 1024 * 1024; // 8MB

        // 1. 检查内存是否足够
        assert!(memory_monitor.can_allocate(chunk_size));

        // 2. 分配内存
        let _guard = MemoryGuard::new(memory_monitor.clone(), chunk_size).unwrap();
        assert_eq!(memory_monitor.current_usage(), chunk_size);

        // 3. 创建上传会话
        let mut session = sessions_mgr
            .create_session(file_path.clone(), file_size)
            .await
            .unwrap();

        assert_eq!(session.file_path, file_path);
        assert_eq!(session.total_size, file_size);

        // 4. 模拟上传过程
        session.status = UploadStatus::Uploading;
        session.update_progress(file_size / 2);
        sessions_mgr.update_session(session.clone()).await.unwrap();

        // 验证活跃会话
        let active_sessions = sessions_mgr.get_active_sessions().await;
        assert_eq!(active_sessions.len(), 1);

        // 5. 完成上传
        session.mark_completed();
        sessions_mgr.update_session(session.clone()).await.unwrap();

        // 6. 添加到秒传索引
        let file_hash = "test_hash_123".to_string();
        instant_upload
            .add_entry(file_hash.clone(), file_size, file_path.clone())
            .await;

        // 7. 验证秒传
        let instant_result = instant_upload
            .check_instant_upload(&file_hash, file_size)
            .await;
        assert!(instant_result.is_some());
        assert_eq!(instant_result.unwrap(), file_path);

        // 8. 清理会话
        sessions_mgr.remove_session(&session.session_id).await;

        // 验证会话已删除
        let retrieved = sessions_mgr.get_session(&session.session_id).await;
        assert!(retrieved.is_none());
    }

    #[tokio::test]
    async fn test_concurrent_uploads_with_memory_limit() {
        // 测试并发上传时的内存管理
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_2");
        let _sessions_mgr = Arc::new(UploadSessionManager::new(temp_dir, 24, 5));
        let memory_monitor = MemoryMonitor::new(50, 80); // 50MB 限制

        let chunk_size = 20 * 1024 * 1024; // 20MB

        // 第一个上传应该成功
        let guard1 = MemoryGuard::new(memory_monitor.clone(), chunk_size);
        assert!(guard1.is_ok());

        // 第二个上传应该成功（总共 40MB < 50MB）
        let guard2 = MemoryGuard::new(memory_monitor.clone(), chunk_size);
        assert!(guard2.is_ok());

        // 第三个上传应该失败（总共 60MB > 50MB）
        let guard3 = MemoryGuard::new(memory_monitor.clone(), chunk_size);
        assert!(guard3.is_err());

        // 验证内存使用量
        assert_eq!(memory_monitor.current_usage(), 40 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_instant_upload_deduplication() {
        // 测试秒传功能的去重效果
        let instant_upload = InstantUploadManager::new();

        let hash = "same_hash_123".to_string();
        let size = 5 * 1024 * 1024; // 5MB

        // 添加第一个文件
        instant_upload
            .add_entry(hash.clone(), size, "/path/file1.txt".to_string())
            .await;

        // 添加第二个相同哈希的文件
        instant_upload
            .add_entry(hash.clone(), size, "/path/file2.txt".to_string())
            .await;

        // 添加第三个相同哈希的文件
        instant_upload
            .add_entry(hash.clone(), size, "/path/file3.txt".to_string())
            .await;

        // 验证秒传返回第一个路径
        let result = instant_upload.check_instant_upload(&hash, size).await;
        assert!(result.is_some());

        // 获取统计信息 (返回 (entry_count, total_size))
        let (entry_count, total_size) = instant_upload.get_stats().await;
        assert_eq!(entry_count, 1); // 只有一个哈希条目
        assert_eq!(total_size, size); // 只计算一次大小
    }

    #[tokio::test]
    async fn test_session_lifecycle() {
        // 测试会话的完整生命周期
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_3");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);

        // 创建会话
        let mut session = sessions_mgr
            .create_session("/test/lifecycle.txt".to_string(), 1000)
            .await
            .unwrap();

        let session_id = session.session_id.clone();

        // 状态转换：Initializing -> Uploading
        session.status = UploadStatus::Uploading;
        sessions_mgr.update_session(session.clone()).await.unwrap();

        let retrieved = sessions_mgr.get_session(&session_id).await.unwrap();
        assert_eq!(retrieved.status, UploadStatus::Uploading);

        // 状态转换：Uploading -> Paused
        session.status = UploadStatus::Paused;
        sessions_mgr.update_session(session.clone()).await.unwrap();

        let retrieved = sessions_mgr.get_session(&session_id).await.unwrap();
        assert_eq!(retrieved.status, UploadStatus::Paused);
        assert!(retrieved.can_resume());

        // 状态转换：Paused -> Uploading
        session.status = UploadStatus::Uploading;
        session.update_progress(500);
        sessions_mgr.update_session(session.clone()).await.unwrap();

        // 状态转换：Uploading -> Completed
        session.mark_completed();
        sessions_mgr.update_session(session.clone()).await.unwrap();

        let retrieved = sessions_mgr.get_session(&session_id).await.unwrap();
        assert_eq!(retrieved.status, UploadStatus::Completed);
        assert_eq!(retrieved.uploaded_size, retrieved.total_size);
        assert_eq!(retrieved.progress_percent(), 100.0);
    }

    #[tokio::test]
    async fn test_session_cleanup() {
        // 测试会话清理功能
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_4");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);

        // 创建一个正常的会话
        let normal_session = sessions_mgr
            .create_session("/test/normal.txt".to_string(), 1000)
            .await
            .unwrap();
        let normal_id = normal_session.session_id.clone();

        // 验证会话存在
        assert!(sessions_mgr.get_session(&normal_id).await.is_some());

        // 清理过期会话（当前没有过期的，应该返回 0）
        let cleaned_count = sessions_mgr.cleanup_expired_sessions().await;
        assert_eq!(cleaned_count, 0);

        // 验证正常会话仍然保留
        assert!(sessions_mgr.get_session(&normal_id).await.is_some());
    }

    #[tokio::test]
    async fn test_concurrent_uploads() {
        // 测试并发上传
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_5");
        let sessions_mgr = Arc::new(UploadSessionManager::new(temp_dir, 24, 5));

        let mut handles = vec![];

        // 创建 3 个并发上传
        for i in 0..3 {
            let mgr = sessions_mgr.clone();
            let handle = tokio::spawn(async move {
                let path = format!("/test/file{}.txt", i);
                let size = (i + 1) as u64 * 1024 * 1024;

                let session = mgr.create_session(path, size).await.unwrap();

                // 模拟上传
                tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

                session.session_id
            });
            handles.push(handle);
        }

        // 等待所有上传完成
        let session_ids: Vec<String> = futures_util::future::join_all(handles)
            .await
            .into_iter()
            .map(|r| r.unwrap())
            .collect();

        // 验证所有会话都被创建
        assert_eq!(session_ids.len(), 3);

        for session_id in session_ids {
            let session = sessions_mgr.get_session(&session_id).await;
            assert!(session.is_some());
        }
    }

    #[tokio::test]
    async fn test_memory_monitor_with_guards() {
        // 测试内存监控器与 RAII 守卫的配合
        let monitor = MemoryMonitor::new(100, 80);

        // 分配和自动释放
        {
            let _guard1 = MemoryGuard::new(monitor.clone(), 30 * 1024 * 1024).unwrap();
            assert_eq!(monitor.current_usage(), 30 * 1024 * 1024);

            {
                let _guard2 = MemoryGuard::new(monitor.clone(), 20 * 1024 * 1024).unwrap();
                assert_eq!(monitor.current_usage(), 50 * 1024 * 1024);
            }

            // guard2 析构后
            assert_eq!(monitor.current_usage(), 30 * 1024 * 1024);
        }

        // 所有守卫析构后
        assert_eq!(monitor.current_usage(), 0);
    }

    #[tokio::test]
    async fn test_instant_upload_with_size_check() {
        // 测试秒传的大小验证
        let instant_upload = InstantUploadManager::new();

        let hash = "test_hash".to_string();
        let original_size = 1024 * 1024; // 1MB

        // 添加原始文件
        instant_upload
            .add_entry(
                hash.clone(),
                original_size,
                "/original/file.txt".to_string(),
            )
            .await;

        // 正确的大小应该匹配
        let result1 = instant_upload
            .check_instant_upload(&hash, original_size)
            .await;
        assert!(result1.is_some());

        // 错误的大小应该不匹配
        let result2 = instant_upload
            .check_instant_upload(&hash, original_size + 1)
            .await;
        assert!(result2.is_none());

        let result3 = instant_upload
            .check_instant_upload(&hash, original_size - 1)
            .await;
        assert!(result3.is_none());
    }

    #[tokio::test]
    async fn test_session_memory_tracking() {
        // 测试会话的内存使用量追踪
        let temp_dir = std::env::temp_dir().join("webdav_integration_test_6");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);

        // 创建几个会话并设置内存使用
        let mut session1 = sessions_mgr
            .create_session("/test/file1.txt".to_string(), 1000)
            .await
            .unwrap();
        session1.memory_usage = 1024 * 1024; // 1MB
        sessions_mgr.update_session(session1).await.unwrap();

        let mut session2 = sessions_mgr
            .create_session("/test/file2.txt".to_string(), 2000)
            .await
            .unwrap();
        session2.memory_usage = 2 * 1024 * 1024; // 2MB
        sessions_mgr.update_session(session2).await.unwrap();

        let mut session3 = sessions_mgr
            .create_session("/test/file3.txt".to_string(), 3000)
            .await
            .unwrap();
        session3.memory_usage = 3 * 1024 * 1024; // 3MB
        sessions_mgr.update_session(session3).await.unwrap();

        // 验证总内存使用量
        let total_memory = sessions_mgr.total_memory_usage().await;
        assert_eq!(total_memory, 6 * 1024 * 1024); // 6MB
    }

    #[test]
    fn test_session_progress_calculation() {
        // 测试进度计算的准确性
        let mut session = UploadSession::new("/test/progress.txt".to_string(), 1000, 24);

        assert_eq!(session.progress_percent(), 0.0);

        session.update_progress(250);
        assert_eq!(session.progress_percent(), 25.0);

        session.update_progress(500);
        assert_eq!(session.progress_percent(), 50.0);

        session.update_progress(750);
        assert_eq!(session.progress_percent(), 75.0);

        session.update_progress(1000);
        assert_eq!(session.progress_percent(), 100.0);
    }

    #[test]
    fn test_session_can_resume_logic() {
        // 测试会话是否可以续传的逻辑
        let mut session = UploadSession::new("/test/resume.txt".to_string(), 1000, 24);

        // Initializing 不能续传
        assert!(!session.can_resume());

        // Uploading 不能续传
        session.status = UploadStatus::Uploading;
        assert!(!session.can_resume());

        // Paused 可以续传
        session.status = UploadStatus::Paused;
        session.uploaded_size = 500;
        assert!(session.can_resume());

        // Failed 可以续传
        session.status = UploadStatus::Failed;
        assert!(session.can_resume());

        // Completed 不能续传
        session.mark_completed();
        assert!(!session.can_resume());

        // Cancelled 不能续传
        session.status = UploadStatus::Cancelled;
        assert!(!session.can_resume());

        // 已过期不能续传
        session.status = UploadStatus::Paused;
        session.expires_at = chrono::Local::now().naive_local() - chrono::Duration::hours(1);
        assert!(!session.can_resume());
    }
}
