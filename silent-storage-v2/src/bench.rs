//! 性能基准测试
//! 对比原版存储与v0.7.0增量存储的性能

#[cfg(test)]
mod tests {
    use crate::{IncrementalConfig, IncrementalStorage};
    use silent_storage_v1::StorageManager;
    use std::sync::Arc;
    use tempfile::TempDir;

    #[test]
    fn test_compression_performance() {
        // 测试数据：重复内容（适合去重和压缩）
        let repetitive_data = b"Hello, World! This is a test. ".repeat(1000);
        let unique_data: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();

        // 1. 测试重复数据
        let compressed_repetitive = compress_data(&repetitive_data);
        println!("\n=== 压缩性能测试 ===");
        println!(
            "重复数据压缩比: {:.2}x",
            repetitive_data.len() as f64 / compressed_repetitive.len() as f64
        );

        // 2. 测试唯一数据
        let compressed_unique = compress_data(&unique_data);
        println!(
            "唯一数据压缩比: {:.2}x",
            unique_data.len() as f64 / compressed_unique.len() as f64
        );
    }

    fn compress_data(data: &[u8]) -> Vec<u8> {
        use lz4_flex::block::compress;
        compress(data)
    }

    #[tokio::test]
    async fn test_incremental_storage_efficiency() {
        let temp_dir = TempDir::new().unwrap();

        // 创建增量存储
        let storage = Arc::new(StorageManager::new(
            temp_dir.path().to_path_buf(),
            4 * 1024 * 1024,
        ));
        storage.init().await.unwrap();

        let config = IncrementalConfig::default();
        let mut incremental =
            IncrementalStorage::new(storage, config, temp_dir.path().to_str().unwrap());
        incremental.init().await.unwrap();

        // 测试场景1：保存新文件
        let data1 = b"Hello, World! This is version 1.".to_vec();
        let (delta1, version1) = incremental
            .save_version("test_file", &data1, None)
            .await
            .unwrap();

        println!("\n=== 增量存储效率测试 ===");
        println!(
            "新文件保存: {} 字节数据，产生 {} 个块",
            data1.len(),
            delta1.chunks.len()
        );

        // 测试场景2：增量更新（修改小部分）
        let mut data2 = data1.clone();
        data2.extend_from_slice(b" This is added content.");
        let (delta2, _version2) = incremental
            .save_version("test_file", &data2, Some(&version1.version_id))
            .await
            .unwrap();

        println!(
            "增量更新: {} 字节数据，产生 {} 个块（vs 完整重新存储）",
            data2.len(),
            delta2.chunks.len()
        );

        // 计算节省空间
        let full_storage_size = data2.len();
        let incremental_storage_size = delta1.chunks.iter().map(|c| c.size).sum::<usize>()
            + delta2.chunks.iter().map(|c| c.size).sum::<usize>();
        let space_saved = full_storage_size as f64 - incremental_storage_size as f64;
        let save_ratio = if full_storage_size > 0 {
            space_saved / full_storage_size as f64 * 100.0
        } else {
            0.0
        };

        println!("\n--- 存储效率分析 ---");
        println!("完整存储: {} 字节", full_storage_size);
        println!("增量存储: {} 字节", incremental_storage_size);
        println!("节省空间: {:.2} 字节 ({:.2}%)", space_saved, save_ratio);

        // 验证数据
        let read_data = incremental
            .read_version_data(&version1.version_id)
            .await
            .unwrap();
        assert_eq!(read_data, data1);
        println!("✓ 数据完整性验证通过");
    }

    #[test]
    fn test_deduplication_ratio() {
        // 模拟场景：多个文件共享相同内容
        let common_chunk = b"This is common content that appears in multiple files. ".to_vec();
        let file1 = [common_chunk.clone(), b"File 1 specific content".to_vec()].concat();
        let file2 = [common_chunk.clone(), b"File 2 specific content".to_vec()].concat();
        let file3 = [common_chunk.clone(), b"File 3 specific content".to_vec()].concat();

        // 计算去重效果
        let total_size = file1.len() + file2.len() + file3.len();
        let deduplicated_size = common_chunk.len()
            + b"File 1 specific content".len()
            + b"File 2 specific content".len()
            + b"File 3 specific content".len();
        let space_saved = total_size - deduplicated_size;
        let dedup_ratio = space_saved as f64 / total_size as f64 * 100.0;

        println!("\n=== 去重效率分析 ===");
        println!("原始总大小: {} 字节", total_size);
        println!("去重后大小: {} 字节", deduplicated_size);
        println!("节省空间: {} 字节 ({:.2}%)", space_saved, dedup_ratio);
    }

    #[test]
    fn test_version_chain_efficiency() {
        // 模拟版本链存储场景
        let base_size = 1000; // 基础版本大小

        // 传统方式：每个版本完整存储
        let version_count = 10;
        let traditional_total = base_size * version_count;

        // 版本链方式：仅存储差异（假设每次修改10%）
        let change_ratio = 0.1;
        let chain_total =
            base_size + (base_size as f32 * change_ratio * (version_count - 1) as f32) as usize;
        let space_saved = traditional_total - chain_total;
        let save_ratio = space_saved as f64 / traditional_total as f64 * 100.0;

        println!("\n=== 版本链存储效率 ===");
        println!("传统完整存储: {} 字节", traditional_total);
        println!("版本链存储: {} 字节", chain_total);
        println!("节省空间: {} 字节 ({:.2}%)", space_saved, save_ratio);
        println!("版本数: {} 个", version_count);
    }
}
