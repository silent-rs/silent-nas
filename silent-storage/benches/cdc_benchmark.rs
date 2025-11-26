use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use silent_storage::IncrementalConfig;
use silent_storage::core::{FileType, RabinKarpChunker};

/// 生成不同类型的测试数据
fn generate_test_data(size: usize, pattern: &str) -> Vec<u8> {
    match pattern {
        "text" => {
            // 模拟文本文件：重复的ASCII文本
            let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. ";
            text.as_bytes().iter().cycle().take(size).copied().collect()
        }
        "binary" => {
            // 模拟二进制文件：伪随机但有模式
            (0..size).map(|i| ((i * 7 + 13) % 256) as u8).collect()
        }
        "repetitive" => {
            // 高重复度数据
            vec![0x42; size]
        }
        "random" => {
            // 低重复度数据（伪随机）
            (0..size)
                .map(|i| {
                    let x = i.wrapping_mul(1103515245).wrapping_add(12345);
                    (x / 65536 % 256) as u8
                })
                .collect()
        }
        _ => vec![0; size],
    }
}

/// 基准测试：不同文件大小的分块性能
fn bench_chunking_by_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunking_by_size");

    let sizes = vec![
        (1024, "1KB"),
        (10 * 1024, "10KB"),
        (100 * 1024, "100KB"),
        (1024 * 1024, "1MB"),
        (10 * 1024 * 1024, "10MB"),
    ];

    for (size, name) in sizes {
        group.throughput(Throughput::Bytes(size as u64));

        let data = generate_test_data(size, "text");

        group.bench_with_input(BenchmarkId::new("text", name), &size, |b, _| {
            b.iter(|| {
                let config = IncrementalConfig::default();
                let mut chunker = RabinKarpChunker::new(config);
                let chunks = chunker.chunk_data(&data).unwrap();
                black_box(chunks);
            });
        });
    }

    group.finish();
}

/// 基准测试：不同数据模式的分块性能
fn bench_chunking_by_pattern(c: &mut Criterion) {
    let mut group = c.benchmark_group("chunking_by_pattern");
    let size = 1024 * 1024; // 1MB
    group.throughput(Throughput::Bytes(size as u64));

    let patterns = vec!["text", "binary", "repetitive", "random"];

    for pattern in patterns {
        let data = generate_test_data(size, pattern);

        group.bench_with_input(BenchmarkId::new("pattern", pattern), pattern, |b, _| {
            b.iter(|| {
                let config = IncrementalConfig::default();
                let mut chunker = RabinKarpChunker::new(config);
                let chunks = chunker.chunk_data(&data).unwrap();
                black_box(chunks);
            });
        });
    }

    group.finish();
}

/// 基准测试：自适应块大小策略的性能影响
fn bench_adaptive_chunk_size(c: &mut Criterion) {
    let mut group = c.benchmark_group("adaptive_chunk_size");
    let size = 1024 * 1024; // 1MB
    group.throughput(Throughput::Bytes(size as u64));

    // 测试文本文件（推荐 2-8KB）
    let text_data = generate_test_data(size, "text");
    let file_type = FileType::Text;
    let (min_chunk, max_chunk) = file_type.recommended_chunk_size();

    // 默认配置
    group.bench_function("text_default", |b| {
        b.iter(|| {
            let config = IncrementalConfig::default();
            let mut chunker = RabinKarpChunker::new(config);
            let chunks = chunker.chunk_data(&text_data).unwrap();
            black_box(chunks);
        });
    });

    // 自适应配置
    group.bench_function("text_adaptive", |b| {
        b.iter(|| {
            let config = IncrementalConfig {
                min_chunk_size: min_chunk,
                max_chunk_size: max_chunk,
                ..Default::default()
            };
            let mut chunker = RabinKarpChunker::new(config);
            let chunks = chunker.chunk_data(&text_data).unwrap();
            black_box(chunks);
        });
    });

    // 测试视频文件（推荐 32-128KB）
    let video_data = generate_test_data(size, "binary");
    let file_type = FileType::Video;
    let (min_chunk, max_chunk) = file_type.recommended_chunk_size();

    group.bench_function("video_default", |b| {
        b.iter(|| {
            let config = IncrementalConfig::default();
            let mut chunker = RabinKarpChunker::new(config);
            let chunks = chunker.chunk_data(&video_data).unwrap();
            black_box(chunks);
        });
    });

    group.bench_function("video_adaptive", |b| {
        b.iter(|| {
            let config = IncrementalConfig {
                min_chunk_size: min_chunk,
                max_chunk_size: max_chunk,
                ..Default::default()
            };
            let mut chunker = RabinKarpChunker::new(config);
            let chunks = chunker.chunk_data(&video_data).unwrap();
            black_box(chunks);
        });
    });

    group.finish();
}

/// 基准测试：去重率评估（统计唯一块数量）
fn bench_deduplication_ratio(c: &mut Criterion) {
    let mut group = c.benchmark_group("deduplication_ratio");
    let size = 1024 * 1024; // 1MB

    let patterns = vec![
        ("text", "高重复文本"),
        ("repetitive", "完全重复数据"),
        ("random", "低重复数据"),
    ];

    for (pattern, desc) in patterns {
        let data = generate_test_data(size, pattern);

        group.bench_function(desc, |b| {
            b.iter(|| {
                let config = IncrementalConfig::default();
                let mut chunker = RabinKarpChunker::new(config);
                let chunks = chunker.chunk_data(&data).unwrap();

                let mut unique_chunks = std::collections::HashSet::new();
                for chunk in &chunks {
                    unique_chunks.insert(&chunk.strong_hash);
                }

                black_box((unique_chunks.len(), chunks.len()));
            });
        });
    }

    group.finish();
}

/// 基准测试：文件类型检测性能
fn bench_file_type_detection(c: &mut Criterion) {
    let mut group = c.benchmark_group("file_type_detection");

    let test_cases = vec![
        (b"\x89PNG\r\n\x1a\n".to_vec(), "PNG魔数"),
        (generate_test_data(1024, "text"), "1KB文本"),
        (generate_test_data(10 * 1024, "binary"), "10KB二进制"),
    ];

    for (data, name) in test_cases {
        group.bench_function(name, |b| {
            b.iter(|| {
                black_box(FileType::detect(&data));
            });
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_chunking_by_size,
    bench_chunking_by_pattern,
    bench_adaptive_chunk_size,
    bench_deduplication_ratio,
    bench_file_type_detection,
);
criterion_main!(benches);
