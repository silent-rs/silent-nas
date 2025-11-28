//! WebDAV 大文件上传性能测试
//!
//! 测试大文件（1GB+）的上传性能，验证内存控制和性能指标

#[cfg(test)]
mod tests {
    use super::super::instant_upload::InstantUploadManager;
    use super::super::memory_monitor::{MemoryGuard, MemoryMonitor};
    use super::super::upload_session::{UploadSessionManager, UploadStatus};
    use std::sync::Arc;
    use std::time::Instant;

    /// 模拟大文件上传，测试内存控制
    #[tokio::test]
    async fn test_large_file_memory_control() {
        let temp_dir = std::env::temp_dir().join("webdav_perf_test_1");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);
        let memory_monitor = MemoryMonitor::new(100, 80); // 100MB 限制

        let file_size = 1024 * 1024 * 1024; // 1GB
        let chunk_size = 8 * 1024 * 1024; // 8MB

        // 创建会话
        let mut session = sessions_mgr
            .create_session("/test/large_file_1gb.bin".to_string(), file_size)
            .await
            .unwrap();

        session.status = UploadStatus::Uploading;

        // 模拟分块上传
        let total_chunks = file_size.div_ceil(chunk_size);
        let mut uploaded = 0u64;

        let start_time = Instant::now();

        for i in 0..total_chunks {
            // 检查内存是否足够
            assert!(
                memory_monitor.can_allocate(chunk_size),
                "内存不足，chunk: {}",
                i
            );

            // 模拟分配内存处理块
            {
                let _guard = MemoryGuard::new(memory_monitor.clone(), chunk_size).unwrap();

                // 模拟处理时间
                tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;

                // 更新进度
                uploaded += chunk_size.min(file_size - uploaded);
            }

            // 守卫析构后，内存应该被释放
            if i < total_chunks - 1 {
                assert_eq!(
                    memory_monitor.current_usage(),
                    0,
                    "内存未正确释放，chunk: {}",
                    i
                );
            }
        }

        let elapsed = start_time.elapsed();

        // 更新会话
        session.update_progress(uploaded);
        session.mark_completed();

        // 验证
        assert_eq!(uploaded, file_size);
        assert_eq!(session.progress_percent(), 100.0);

        // 输出性能指标
        println!("\n=== 大文件上传性能测试 (1GB) ===");
        println!("文件大小: {} MB", file_size / 1024 / 1024);
        println!("块大小: {} MB", chunk_size / 1024 / 1024);
        println!("总块数: {}", total_chunks);
        println!("总耗时: {:.2} 秒", elapsed.as_secs_f64());
        println!(
            "模拟速度: {:.2} MB/s",
            (file_size as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64()
        );
        println!("峰值内存: {} MB", memory_monitor.limit_mb());
        println!("最终内存使用: {} MB", memory_monitor.current_usage_mb());
    }

    /// 测试多个大文件并发上传
    #[tokio::test]
    async fn test_concurrent_large_files() {
        let temp_dir = std::env::temp_dir().join("webdav_perf_test_2");
        let sessions_mgr = Arc::new(UploadSessionManager::new(temp_dir, 24, 5));
        let memory_monitor = Arc::new(MemoryMonitor::new(100, 80));

        let file_size = 512 * 1024 * 1024; // 512MB 每个文件
        let chunk_size = 8 * 1024 * 1024; // 8MB
        let concurrent_uploads = 3;

        let start_time = Instant::now();
        let mut handles = vec![];

        for i in 0..concurrent_uploads {
            let mgr = sessions_mgr.clone();
            let monitor = memory_monitor.clone();

            let handle = tokio::spawn(async move {
                let path = format!("/test/large_file_{}_512mb.bin", i);

                // 创建会话
                let mut session = mgr.create_session(path, file_size).await.unwrap();
                session.status = UploadStatus::Uploading;

                // 模拟分块上传
                let total_chunks = file_size.div_ceil(chunk_size);
                let mut uploaded = 0u64;

                for _ in 0..total_chunks {
                    // 尝试分配内存
                    loop {
                        if monitor.can_allocate(chunk_size) {
                            break;
                        }
                        // 等待内存可用
                        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
                    }

                    // 处理块
                    {
                        let _guard = MemoryGuard::new((*monitor).clone(), chunk_size).unwrap();
                        tokio::time::sleep(tokio::time::Duration::from_micros(100)).await;
                        uploaded += chunk_size.min(file_size - uploaded);
                    }
                }

                session.update_progress(uploaded);
                session.mark_completed();

                uploaded
            });

            handles.push(handle);
        }

        // 等待所有上传完成
        let results = futures_util::future::join_all(handles).await;

        let elapsed = start_time.elapsed();

        // 验证所有上传都成功
        for (i, result) in results.iter().enumerate() {
            assert_eq!(result.as_ref().unwrap(), &file_size, "上传 {} 失败", i);
        }

        let total_size = file_size * concurrent_uploads;

        println!("\n=== 并发大文件上传性能测试 ===");
        println!("并发数: {}", concurrent_uploads);
        println!("单文件大小: {} MB", file_size / 1024 / 1024);
        println!("总大小: {} MB", total_size / 1024 / 1024);
        println!("总耗时: {:.2} 秒", elapsed.as_secs_f64());
        println!(
            "聚合速度: {:.2} MB/s",
            (total_size as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64()
        );
        println!("内存限制: {} MB", memory_monitor.limit_mb());
        println!("最终内存使用: {:.2} MB", memory_monitor.current_usage_mb());
    }

    /// 测试超大文件（2GB）上传
    #[tokio::test]
    #[ignore] // 标记为 ignore，因为测试时间较长
    async fn test_very_large_file_2gb() {
        let temp_dir = std::env::temp_dir().join("webdav_perf_test_3");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 10);
        let memory_monitor = MemoryMonitor::new(100, 80);

        let file_size = 2 * 1024 * 1024 * 1024; // 2GB
        let chunk_size = 8 * 1024 * 1024; // 8MB

        let mut session = sessions_mgr
            .create_session("/test/very_large_file_2gb.bin".to_string(), file_size)
            .await
            .unwrap();

        session.status = UploadStatus::Uploading;

        let total_chunks = file_size.div_ceil(chunk_size);
        let mut uploaded = 0u64;

        let start_time = Instant::now();
        let mut last_report_time = start_time;
        let mut last_uploaded = 0u64;

        for i in 0..total_chunks {
            {
                let _guard = MemoryGuard::new(memory_monitor.clone(), chunk_size).unwrap();
                tokio::time::sleep(tokio::time::Duration::from_micros(50)).await;
                uploaded += chunk_size.min(file_size - uploaded);
            }

            // 每 10% 报告一次进度
            if (i + 1) % (total_chunks / 10) == 0 || i == total_chunks - 1 {
                let now = Instant::now();
                let elapsed = now.duration_since(last_report_time).as_secs_f64();
                let chunk_uploaded = uploaded - last_uploaded;
                let speed = (chunk_uploaded as f64 / 1024.0 / 1024.0) / elapsed;

                println!(
                    "进度: {:.1}% ({}/{} MB) - 速度: {:.2} MB/s",
                    ((uploaded as f64 / file_size as f64) * 100.0),
                    uploaded / 1024 / 1024,
                    file_size / 1024 / 1024,
                    speed
                );

                last_report_time = now;
                last_uploaded = uploaded;
            }
        }

        let elapsed = start_time.elapsed();

        session.update_progress(uploaded);
        session.mark_completed();

        println!("\n=== 超大文件上传性能测试 (2GB) ===");
        println!("文件大小: {} MB", file_size / 1024 / 1024);
        println!("总耗时: {:.2} 秒", elapsed.as_secs_f64());
        println!(
            "平均速度: {:.2} MB/s",
            (file_size as f64 / 1024.0 / 1024.0) / elapsed.as_secs_f64()
        );
        println!("内存限制: {} MB", memory_monitor.limit_mb());
        println!("内存使用峰值: {:.2} MB", memory_monitor.current_usage_mb());
    }

    /// 测试会话管理性能
    #[tokio::test]
    async fn test_session_management_performance() {
        let temp_dir = std::env::temp_dir().join("webdav_perf_test_4");
        let sessions_mgr = UploadSessionManager::new(temp_dir, 24, 100);

        let num_sessions = 1000;
        let start_time = Instant::now();

        // 创建大量会话
        let mut session_ids = Vec::new();
        for i in 0..num_sessions {
            let session = sessions_mgr
                .create_session(format!("/test/file_{}.bin", i), 1024 * 1024)
                .await
                .unwrap();
            session_ids.push(session.session_id);
        }

        let create_elapsed = start_time.elapsed();

        // 查询所有会话
        let query_start = Instant::now();
        for session_id in &session_ids {
            let session = sessions_mgr.get_session(session_id).await;
            assert!(session.is_some());
        }
        let query_elapsed = query_start.elapsed();

        // 更新所有会话
        let update_start = Instant::now();
        for session_id in &session_ids {
            let mut session = sessions_mgr.get_session(session_id).await.unwrap();
            session.update_progress(512 * 1024);
            sessions_mgr.update_session(session).await.unwrap();
        }
        let update_elapsed = update_start.elapsed();

        // 删除所有会话
        let delete_start = Instant::now();
        for session_id in &session_ids {
            sessions_mgr.remove_session(session_id).await;
        }
        let delete_elapsed = delete_start.elapsed();

        println!("\n=== 会话管理性能测试 ===");
        println!("会话数量: {}", num_sessions);
        println!(
            "创建耗时: {:.3} 秒 ({:.0} 会话/秒)",
            create_elapsed.as_secs_f64(),
            num_sessions as f64 / create_elapsed.as_secs_f64()
        );
        println!(
            "查询耗时: {:.3} 秒 ({:.0} 查询/秒)",
            query_elapsed.as_secs_f64(),
            num_sessions as f64 / query_elapsed.as_secs_f64()
        );
        println!(
            "更新耗时: {:.3} 秒 ({:.0} 更新/秒)",
            update_elapsed.as_secs_f64(),
            num_sessions as f64 / update_elapsed.as_secs_f64()
        );
        println!(
            "删除耗时: {:.3} 秒 ({:.0} 删除/秒)",
            delete_elapsed.as_secs_f64(),
            num_sessions as f64 / delete_elapsed.as_secs_f64()
        );
    }

    /// 测试秒传索引性能
    #[tokio::test]
    async fn test_instant_upload_index_performance() {
        let instant_upload = InstantUploadManager::new();

        let num_entries = 10000;
        let start_time = Instant::now();

        // 添加大量条目
        for i in 0..num_entries {
            let hash = format!("hash_{:08x}", i);
            instant_upload
                .add_entry(hash, 1024 * 1024, format!("/test/file_{}.bin", i))
                .await;
        }

        let add_elapsed = start_time.elapsed();

        // 查询所有条目
        let query_start = Instant::now();
        for i in 0..num_entries {
            let hash = format!("hash_{:08x}", i);
            let result = instant_upload
                .check_instant_upload(&hash, 1024 * 1024)
                .await;
            assert!(result.is_some());
        }
        let query_elapsed = query_start.elapsed();

        // 获取统计信息
        let (entry_count, total_size) = instant_upload.get_stats().await;

        println!("\n=== 秒传索引性能测试 ===");
        println!("条目数量: {}", num_entries);
        println!(
            "添加耗时: {:.3} 秒 ({:.0} 条目/秒)",
            add_elapsed.as_secs_f64(),
            num_entries as f64 / add_elapsed.as_secs_f64()
        );
        println!(
            "查询耗时: {:.3} 秒 ({:.0} 查询/秒)",
            query_elapsed.as_secs_f64(),
            num_entries as f64 / query_elapsed.as_secs_f64()
        );
        println!("索引条目数: {}", entry_count);
        println!("索引总大小: {} MB", total_size / 1024 / 1024);
    }

    /// 测试内存监控器性能
    #[test]
    fn test_memory_monitor_performance() {
        let monitor = MemoryMonitor::new(1000, 80); // 1000MB

        let num_operations = 100000;
        let chunk_size = 8 * 1024 * 1024; // 8MB

        let start_time = Instant::now();

        for _ in 0..num_operations {
            // 分配和释放
            monitor.allocate(chunk_size).unwrap();
            monitor.release(chunk_size);
        }

        let elapsed = start_time.elapsed();

        println!("\n=== 内存监控器性能测试 ===");
        println!("操作次数: {}", num_operations);
        println!("每次操作: {} MB", chunk_size / 1024 / 1024);
        println!("总耗时: {:.3} 秒", elapsed.as_secs_f64());
        println!(
            "操作速度: {:.0} 操作/秒",
            num_operations as f64 / elapsed.as_secs_f64()
        );
        println!(
            "平均延迟: {:.3} μs",
            (elapsed.as_micros() as f64) / (num_operations as f64)
        );
    }
}
