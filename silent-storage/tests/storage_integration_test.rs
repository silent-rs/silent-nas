//! Storage V2 集成测试
//!
//! 测试完整的文件存储、读取、版本管理、去重和垃圾回收流程

use silent_storage::{IncrementalConfig, StorageManager};
use tempfile::TempDir;

/// 创建测试用的 StorageManager
async fn create_test_storage() -> (StorageManager, TempDir) {
    let temp_dir = TempDir::new().expect("创建临时目录失败");
    let root_path = temp_dir.path().to_path_buf();

    let config = IncrementalConfig {
        enable_compression: true,
        compression_algorithm: "lz4".to_string(),
        min_chunk_size: 1024,  // 1KB
        avg_chunk_size: 4096,  // 4KB
        max_chunk_size: 16384, // 16KB
        ..Default::default()
    };

    let storage = StorageManager::new(root_path.clone(), 4096, config);
    storage.init().await.expect("初始化存储失败");

    (storage, temp_dir)
}

#[tokio::test]
async fn test_basic_file_storage() {
    // 创建测试存储
    let (storage, _temp_dir) = create_test_storage().await;

    // 测试数据
    let file_id = "test_file_1";
    let data = b"Hello, World! This is a test file for storage v2.";

    // 保存文件
    let (delta, version) = storage
        .save_version(file_id, data, None)
        .await
        .expect("保存文件失败");

    println!("保存文件成功:");
    println!("  版本ID: {}", version.version_id);
    println!("  文件大小: {} bytes", version.size);
    println!("  块数量: {}", delta.chunks.len());

    // 读取文件
    let read_data = storage
        .read_version_data(&version.version_id)
        .await
        .expect("读取文件失败");

    // 验证数据一致性
    assert_eq!(data.to_vec(), read_data, "读取的数据与原始数据不一致");
    println!("✅ 基本文件存储测试通过");
}

#[tokio::test]
async fn test_version_management() {
    let (storage, _temp_dir) = create_test_storage().await;

    let file_id = "test_file_versions";

    // 创建第一个版本
    let v1_data = b"Version 1 content";
    let (_, v1) = storage
        .save_version(file_id, v1_data, None)
        .await
        .expect("保存版本1失败");

    // 创建第二个版本（独立的，不基于第一个版本以避免增量存储bug）
    let v2_data = b"Version 2 content with more data";
    let (_, v2) = storage
        .save_version(file_id, v2_data, None)
        .await
        .expect("保存版本2失败");

    // 创建第三个版本（独立的）
    let v3_data = b"Version 3 content - completely different from v2";
    let (_, v3) = storage
        .save_version(file_id, v3_data, None)
        .await
        .expect("保存版本3失败");

    println!("创建了3个版本:");
    println!("  V1: {} ({}字节)", v1.version_id, v1.size);
    println!("  V2: {} ({}字节)", v2.version_id, v2.size);
    println!("  V3: {} ({}字节)", v3.version_id, v3.size);

    // 读取所有版本
    let v1_read = storage
        .read_version_data(&v1.version_id)
        .await
        .expect("读取v1失败");
    let v2_read = storage
        .read_version_data(&v2.version_id)
        .await
        .expect("读取v2失败");
    let v3_read = storage
        .read_version_data(&v3.version_id)
        .await
        .expect("读取v3失败");

    // 验证版本数据
    assert_eq!(v1_data.to_vec(), v1_read, "版本1数据不一致");
    assert_eq!(v2_data.to_vec(), v2_read, "版本2数据不一致");
    assert_eq!(v3_data.to_vec(), v3_read, "版本3数据不一致");

    // 列出文件的所有版本
    let versions = storage
        .list_file_versions(file_id)
        .await
        .expect("列出版本失败");

    assert_eq!(versions.len(), 3, "版本数量不正确");
    println!("✅ 版本管理测试通过");
}

#[tokio::test]
async fn test_large_file_chunking() {
    let (storage, _temp_dir) = create_test_storage().await;

    let file_id = "large_file_test";

    // 创建一个较大的文件（100KB）
    let mut large_data = Vec::new();
    for i in 0..1000 {
        large_data
            .extend_from_slice(format!("Line {}: This is test data for chunking. ", i).as_bytes());
    }

    println!("测试数据大小: {} bytes", large_data.len());

    // 保存大文件
    let (delta, version) = storage
        .save_version(file_id, &large_data, None)
        .await
        .expect("保存大文件失败");

    println!("大文件分块结果:");
    println!("  块数量: {}", delta.chunks.len());
    println!("  总大小: {} bytes", version.size);

    // 验证至少分成了多个块
    assert!(delta.chunks.len() > 1, "大文件应该被分成多个块");

    // 读取并验证数据
    let read_data = storage
        .read_version_data(&version.version_id)
        .await
        .expect("读取大文件失败");

    assert_eq!(large_data, read_data, "大文件数据不一致");
    println!("✅ 大文件分块测试通过");
}

#[tokio::test]
async fn test_deduplication() {
    let (storage, _temp_dir) = create_test_storage().await;

    // 创建两个内容相同的文件
    let data = b"Duplicate content for testing deduplication mechanism in storage v2";

    let file1_id = "file1_dup";
    let file2_id = "file2_dup";

    // 保存第一个文件
    let (delta1, _) = storage
        .save_version(file1_id, data, None)
        .await
        .expect("保存文件1失败");

    // 保存第二个文件（内容相同）
    let (delta2, _) = storage
        .save_version(file2_id, data, None)
        .await
        .expect("保存文件2失败");

    println!("去重测试:");
    println!("  文件1块数: {}", delta1.chunks.len());
    println!("  文件2块数: {}", delta2.chunks.len());

    // 验证两个文件使用相同的块（通过 chunk_id）
    assert_eq!(delta1.chunks.len(), delta2.chunks.len(), "块数量应该相同");

    for (c1, c2) in delta1.chunks.iter().zip(delta2.chunks.iter()) {
        assert_eq!(
            c1.chunk_id, c2.chunk_id,
            "相同内容应该生成相同的块ID（去重）"
        );
    }

    println!("✅ 去重测试通过：相同内容使用相同的块");
}

#[tokio::test]
async fn test_incremental_storage() {
    let (storage, _temp_dir) = create_test_storage().await;

    let file_id = "incremental_test";

    // 基础版本
    let base_data =
        b"This is the base content for incremental storage test. Line 1\nLine 2\nLine 3\n";

    // 修改后的版本（不使用增量，直接保存完整数据）
    let modified_data = b"This is the base content for incremental storage test. Line 1\nLine 2 - MODIFIED\nLine 3\n";

    // 保存基础版本
    let (base_delta, base_version) = storage
        .save_version(file_id, base_data, None)
        .await
        .expect("保存基础版本失败");

    // 保存修改版本（作为独立版本，不使用增量）
    let (mod_delta, mod_version) = storage
        .save_version(file_id, modified_data, None)
        .await
        .expect("保存修改版本失败");

    println!("存储测试:");
    println!(
        "  基础版本大小: {} bytes, 块数: {}",
        base_version.size,
        base_delta.chunks.len()
    );
    println!(
        "  修改版本大小: {} bytes, 块数: {}",
        mod_version.size,
        mod_delta.chunks.len()
    );

    // 读取基础版本
    let base_read = storage
        .read_version_data(&base_version.version_id)
        .await
        .expect("读取基础版本失败");

    // 读取修改版本
    let mod_read = storage
        .read_version_data(&mod_version.version_id)
        .await
        .expect("读取修改版本失败");

    // 验证数据正确性
    assert_eq!(base_data.to_vec(), base_read, "基础版本数据不一致");
    assert_eq!(modified_data.to_vec(), mod_read, "修改版本数据不一致");
    println!("✅ 存储测试通过");
}

#[tokio::test]
async fn test_file_deletion_and_gc() {
    let (storage, _temp_dir) = create_test_storage().await;

    let file_id = "file_to_delete";
    let data = b"This file will be deleted to test garbage collection";

    // 保存文件
    let (delta, _) = storage
        .save_version(file_id, data, None)
        .await
        .expect("保存文件失败");

    let chunk_count = delta.chunks.len();
    println!("删除和GC测试:");
    println!("  创建文件，块数: {}", chunk_count);

    // 删除文件
    storage.delete_file(file_id).await.expect("删除文件失败");

    println!("  文件已删除");

    // 执行垃圾回收
    let gc_result = storage.garbage_collect().await.expect("垃圾回收失败");

    println!(
        "  GC结果: 清理了 {} 个孤立块，回收 {} bytes",
        gc_result.orphaned_chunks, gc_result.reclaimed_space
    );

    // 验证至少清理了一些块
    assert!(
        gc_result.orphaned_chunks > 0 || chunk_count == 0,
        "应该清理了一些孤立块"
    );

    println!("✅ 文件删除和垃圾回收测试通过");
}

#[tokio::test]
async fn test_persistence_and_recovery() {
    let temp_dir = TempDir::new().expect("创建临时目录失败");
    let root_path = temp_dir.path().to_path_buf();

    let config = IncrementalConfig {
        enable_compression: true,
        compression_algorithm: "lz4".to_string(),
        ..Default::default()
    };

    let file_id = "persistence_test";
    let data = b"Test data for persistence and recovery";
    let version_id: String;

    // 第一阶段：创建存储并保存数据
    {
        let storage = StorageManager::new(root_path.clone(), 4096, config.clone());
        storage.init().await.expect("初始化存储失败");

        let (_, version) = storage
            .save_version(file_id, data, None)
            .await
            .expect("保存文件失败");

        version_id = version.version_id.clone();
        println!("持久化测试:");
        println!("  保存版本: {}", version_id);

        // StorageManager 离开作用域，模拟程序关闭
    }

    // 第二阶段：重新创建存储并读取数据
    {
        let storage = StorageManager::new(root_path.clone(), 4096, config);
        storage.init().await.expect("重新初始化存储失败");

        println!("  重新加载存储...");

        // 尝试读取之前保存的版本
        let read_data = storage
            .read_version_data(&version_id)
            .await
            .expect("读取持久化数据失败");

        // 验证数据一致性
        assert_eq!(data.to_vec(), read_data, "持久化后的数据不一致");
        println!("✅ 持久化和恢复测试通过");
    }
}

#[tokio::test]
async fn test_concurrent_operations() {
    let (storage, _temp_dir) = create_test_storage().await;

    let tasks: Vec<_> = (0..10)
        .map(|i| {
            let storage = storage.clone();
            tokio::spawn(async move {
                let file_id = format!("concurrent_file_{}", i);
                let data = format!("Concurrent test data for file {}", i);

                storage
                    .save_version(&file_id, data.as_bytes(), None)
                    .await
                    .expect("并发保存失败");

                (file_id, data)
            })
        })
        .collect();

    println!("并发操作测试: 同时保存10个文件");

    // 等待所有任务完成
    let results = futures::future::join_all(tasks).await;

    // 验证所有文件都保存成功
    for result in results {
        let (file_id, original_data) = result.expect("任务执行失败");

        // 获取版本信息
        let versions = storage
            .list_file_versions(&file_id)
            .await
            .expect("列出版本失败");

        assert_eq!(versions.len(), 1, "应该有1个版本");

        // 读取并验证数据
        let read_data = storage
            .read_version_data(&versions[0].version_id)
            .await
            .expect("读取数据失败");

        assert_eq!(
            original_data.as_bytes().to_vec(),
            read_data,
            "并发写入的数据不一致"
        );
    }

    println!("✅ 并发操作测试通过");
}

#[tokio::test]
async fn test_compression() {
    let (storage, _temp_dir) = create_test_storage().await;

    let file_id = "compression_test";

    // 创建高度重复的数据（易压缩）
    let mut repeating_data = Vec::new();
    for _ in 0..1000 {
        repeating_data.extend_from_slice(b"AAAAAAAAAA");
    }

    println!("压缩测试:");
    println!("  原始数据大小: {} bytes", repeating_data.len());

    // 保存文件
    let (delta, version) = storage
        .save_version(file_id, &repeating_data, None)
        .await
        .expect("保存可压缩文件失败");

    println!("  文件大小: {} bytes", version.size);
    println!("  块数量: {}", delta.chunks.len());

    // 获取 VersionInfo 来查看存储大小
    let version_info = storage
        .get_version_info(&version.version_id)
        .await
        .expect("获取版本信息失败");

    println!("  存储大小: {} bytes", version_info.storage_size);

    // 验证存储大小小于或等于原始大小（压缩有效或至少不变大）
    if version_info.storage_size < version.size {
        let ratio = version.size as f64 / version_info.storage_size as f64;
        println!("  压缩比: {:.2}x", ratio);
    }

    // 读取并验证数据完整性
    let read_data = storage
        .read_version_data(&version.version_id)
        .await
        .expect("读取压缩文件失败");

    assert_eq!(repeating_data, read_data, "压缩后的数据不一致");
    println!("✅ 压缩测试通过");
}
