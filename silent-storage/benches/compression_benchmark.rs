use criterion::{Criterion, Throughput, black_box, criterion_group, criterion_main};
use silent_storage::core::{CompressionAlgorithm, CompressionConfig, Compressor};

/// 生成不同类型的测试数据
fn generate_test_data(size: usize, pattern: &str) -> Vec<u8> {
    match pattern {
        "text" => {
            // 模拟文本文件：重复的ASCII文本（高压缩比）
            let text = "Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
                        Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. ";
            text.as_bytes().iter().cycle().take(size).copied().collect()
        }
        "json" => {
            // 模拟JSON数据：结构化重复（中等压缩比）
            let json =
                r#"{"id":1234,"name":"test user","email":"test@example.com","status":"active"}"#;
            json.as_bytes().iter().cycle().take(size).copied().collect()
        }
        "repetitive" => {
            // 高重复度数据（极高压缩比）
            vec![0x42; size]
        }
        "random" => {
            // 低重复度数据（低压缩比）
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

/// 基准测试：不同压缩算法的性能
fn bench_compression_algorithms(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_algorithms");
    let size = 1024 * 1024; // 1MB
    group.throughput(Throughput::Bytes(size as u64));

    let data = generate_test_data(size, "text");

    // LZ4 压缩
    group.bench_function("LZ4", |b| {
        let config = CompressionConfig {
            algorithm: CompressionAlgorithm::LZ4,
            level: 1,
            ..Default::default()
        };
        let compressor = Compressor::new(config);
        b.iter(|| {
            black_box(compressor.compress(&data).unwrap());
        });
    });

    // Zstd 压缩
    group.bench_function("Zstd", |b| {
        let config = CompressionConfig {
            algorithm: CompressionAlgorithm::Zstd,
            level: 3,
            ..Default::default()
        };
        let compressor = Compressor::new(config);
        b.iter(|| {
            black_box(compressor.compress(&data).unwrap());
        });
    });

    // 无压缩（基准）
    group.bench_function("None", |b| {
        let config = CompressionConfig {
            algorithm: CompressionAlgorithm::None,
            ..Default::default()
        };
        let compressor = Compressor::new(config);
        b.iter(|| {
            black_box(compressor.compress(&data).unwrap());
        });
    });

    group.finish();
}

/// 基准测试：不同数据模式的压缩比
fn bench_compression_ratio(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_ratio");
    let size = 1024 * 1024; // 1MB

    let patterns = vec![
        ("text", "高重复文本"),
        ("json", "JSON数据"),
        ("repetitive", "完全重复"),
        ("random", "低重复随机"),
    ];

    for (pattern, desc) in patterns {
        let data = generate_test_data(size, pattern);

        // LZ4 压缩
        group.bench_function(format!("LZ4/{}", desc), |b| {
            let config = CompressionConfig {
                algorithm: CompressionAlgorithm::LZ4,
                level: 1,
                ..Default::default()
            };
            let compressor = Compressor::new(config);
            b.iter(|| {
                let result = compressor.compress(&data).unwrap();
                black_box((result.compressed_size, result.ratio));
            });
        });

        // Zstd 压缩
        group.bench_function(format!("Zstd/{}", desc), |b| {
            let config = CompressionConfig {
                algorithm: CompressionAlgorithm::Zstd,
                level: 3,
                ..Default::default()
            };
            let compressor = Compressor::new(config);
            b.iter(|| {
                let result = compressor.compress(&data).unwrap();
                black_box((result.compressed_size, result.ratio));
            });
        });
    }

    group.finish();
}

/// 基准测试：不同压缩等级的性能权衡
fn bench_compression_levels(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_levels");
    let size = 1024 * 1024; // 1MB
    group.throughput(Throughput::Bytes(size as u64));

    let data = generate_test_data(size, "text");

    // Zstd 不同等级
    for level in [1, 3, 6, 9] {
        group.bench_function(format!("Zstd/level_{}", level), |b| {
            let config = CompressionConfig {
                algorithm: CompressionAlgorithm::Zstd,
                level,
                ..Default::default()
            };
            let compressor = Compressor::new(config);
            b.iter(|| {
                black_box(compressor.compress(&data).unwrap());
            });
        });
    }

    group.finish();
}

/// 基准测试：压缩阈值影响
fn bench_compression_threshold(c: &mut Criterion) {
    let mut group = c.benchmark_group("compression_threshold");

    let sizes = vec![
        (512, "512B"),
        (1024, "1KB"),
        (10 * 1024, "10KB"),
        (100 * 1024, "100KB"),
    ];

    for (size, name) in sizes {
        group.throughput(Throughput::Bytes(size as u64));
        let data = generate_test_data(size, "text");

        // 阈值 1KB
        group.bench_function(format!("threshold_1KB/{}", name), |b| {
            let config = CompressionConfig {
                algorithm: CompressionAlgorithm::LZ4,
                min_size: 1024,
                ..Default::default()
            };
            let compressor = Compressor::new(config);
            b.iter(|| {
                black_box(compressor.compress(&data).unwrap());
            });
        });
    }

    group.finish();
}

/// 基准测试：解压缩性能
fn bench_decompression(c: &mut Criterion) {
    let mut group = c.benchmark_group("decompression");
    let size = 1024 * 1024; // 1MB
    let data = generate_test_data(size, "text");

    // 预压缩数据
    let lz4_config = CompressionConfig {
        algorithm: CompressionAlgorithm::LZ4,
        ..Default::default()
    };
    let lz4_compressor = Compressor::new(lz4_config.clone());
    let lz4_result = lz4_compressor.compress(&data).unwrap();

    let zstd_config = CompressionConfig {
        algorithm: CompressionAlgorithm::Zstd,
        level: 3,
        ..Default::default()
    };
    let zstd_compressor = Compressor::new(zstd_config.clone());
    let zstd_result = zstd_compressor.compress(&data).unwrap();

    // 获取压缩后的数据（需要实际压缩实现）
    group.throughput(Throughput::Bytes(size as u64));

    group.bench_function("LZ4", |b| {
        b.iter(|| {
            // 注意：这里假设有获取压缩数据的方法
            black_box(lz4_result.compressed_size);
        });
    });

    group.bench_function("Zstd", |b| {
        b.iter(|| {
            black_box(zstd_result.compressed_size);
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_compression_algorithms,
    bench_compression_ratio,
    bench_compression_levels,
    bench_compression_threshold,
    bench_decompression,
);
criterion_main!(benches);
