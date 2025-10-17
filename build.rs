fn main() -> Result<(), Box<dyn std::error::Error>> {
    // tonic-prost-build 0.14 官方推荐用法
    // 参考: https://docs.rs/tonic-build/latest/tonic_build/
    tonic_prost_build::configure().compile_protos(&["proto/file_service.proto"], &["proto"])?;

    Ok(())
}
