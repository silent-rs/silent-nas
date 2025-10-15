use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub nats: NatsConfig,
    pub s3: S3Config,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub http_port: u16,
    pub grpc_port: u16,
    pub quic_port: u16,
    pub webdav_port: u16,
    pub s3_port: u16,
    pub host: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageConfig {
    pub root_path: PathBuf,
    pub chunk_size: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NatsConfig {
    pub url: String,
    pub topic_prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct S3Config {
    pub access_key: String,
    pub secret_key: String,
    pub enable_auth: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: ServerConfig {
                http_port: 8080,
                grpc_port: 50051,
                quic_port: 4433,
                webdav_port: 8081,
                s3_port: 9000,
                host: "127.0.0.1".to_string(),
            },
            storage: StorageConfig {
                root_path: PathBuf::from("./storage"),
                chunk_size: 4 * 1024 * 1024, // 4MB
            },
            nats: NatsConfig {
                url: "nats://127.0.0.1:4222".to_string(),
                topic_prefix: "silent.nas.files".to_string(),
            },
            s3: S3Config {
                access_key: "minioadmin".to_string(),
                secret_key: "minioadmin".to_string(),
                enable_auth: false,
            },
        }
    }
}

impl Config {
    pub fn from_file(path: &str) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| NasError::Config(format!("无法读取配置文件: {}", e)))?;
        let config: Config = toml::from_str(&content)
            .map_err(|e| NasError::Config(format!("配置文件解析失败: {}", e)))?;
        Ok(config)
    }

    pub fn load() -> Self {
        Self::from_file("config.toml").unwrap_or_default()
    }
}
