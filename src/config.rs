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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_config_default() {
        let config = Config::default();

        // 测试服务器配置
        assert_eq!(config.server.http_port, 8080);
        assert_eq!(config.server.grpc_port, 50051);
        assert_eq!(config.server.quic_port, 4433);
        assert_eq!(config.server.webdav_port, 8081);
        assert_eq!(config.server.s3_port, 9000);
        assert_eq!(config.server.host, "127.0.0.1");

        // 测试存储配置
        assert_eq!(config.storage.root_path, PathBuf::from("./storage"));
        assert_eq!(config.storage.chunk_size, 4 * 1024 * 1024);

        // 测试NATS配置
        assert_eq!(config.nats.url, "nats://127.0.0.1:4222");
        assert_eq!(config.nats.topic_prefix, "silent.nas.files");

        // 测试S3配置
        assert_eq!(config.s3.access_key, "minioadmin");
        assert_eq!(config.s3.secret_key, "minioadmin");
        assert!(!config.s3.enable_auth);
    }

    #[test]
    fn test_server_config() {
        let server = ServerConfig {
            http_port: 9090,
            grpc_port: 50052,
            quic_port: 4434,
            webdav_port: 8082,
            s3_port: 9001,
            host: "0.0.0.0".to_string(),
        };

        assert_eq!(server.http_port, 9090);
        assert_eq!(server.host, "0.0.0.0");
    }

    #[test]
    fn test_storage_config() {
        let storage = StorageConfig {
            root_path: PathBuf::from("/tmp/storage"),
            chunk_size: 8 * 1024 * 1024,
        };

        assert_eq!(storage.root_path, PathBuf::from("/tmp/storage"));
        assert_eq!(storage.chunk_size, 8 * 1024 * 1024);
    }

    #[test]
    fn test_nats_config() {
        let nats = NatsConfig {
            url: "nats://localhost:4222".to_string(),
            topic_prefix: "test.prefix".to_string(),
        };

        assert_eq!(nats.url, "nats://localhost:4222");
        assert_eq!(nats.topic_prefix, "test.prefix");
    }

    #[test]
    fn test_s3_config() {
        let s3 = S3Config {
            access_key: "test_key".to_string(),
            secret_key: "test_secret".to_string(),
            enable_auth: true,
        };

        assert_eq!(s3.access_key, "test_key");
        assert_eq!(s3.secret_key, "test_secret");
        assert!(s3.enable_auth);
    }

    #[test]
    fn test_config_serialization() {
        let config = Config::default();

        // 测试序列化为TOML
        let toml_str = toml::to_string(&config).unwrap();
        assert!(toml_str.contains("http_port"));
        assert!(toml_str.contains("8080"));

        // 测试反序列化
        let deserialized: Config = toml::from_str(&toml_str).unwrap();
        assert_eq!(deserialized.server.http_port, 8080);
    }

    #[test]
    fn test_config_from_file_not_found() {
        let result = Config::from_file("non_existent_file.toml");
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("无法读取配置文件"));
        }
    }

    #[test]
    fn test_config_from_file_invalid_toml() {
        // 创建临时配置文件
        let temp_file = "./test_invalid_config.toml";
        fs::write(temp_file, "invalid toml content [[[").unwrap();

        let result = Config::from_file(temp_file);
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("配置文件解析失败"));
        }

        // 清理
        let _ = fs::remove_file(temp_file);
    }

    #[test]
    fn test_config_from_file_valid() {
        // 创建临时配置文件
        let temp_file = "./test_valid_config.toml";
        let config_content = r#"
[server]
http_port = 9999
grpc_port = 50053
quic_port = 4435
webdav_port = 8083
s3_port = 9002
host = "0.0.0.0"

[storage]
root_path = "/tmp/test_storage"
chunk_size = 8388608

[nats]
url = "nats://test:4222"
topic_prefix = "test.topic"

[s3]
access_key = "testkey"
secret_key = "testsecret"
enable_auth = true
"#;
        fs::write(temp_file, config_content).unwrap();

        let config = Config::from_file(temp_file).unwrap();
        assert_eq!(config.server.http_port, 9999);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.storage.chunk_size, 8388608);
        assert_eq!(config.nats.url, "nats://test:4222");
        assert_eq!(config.s3.access_key, "testkey");
        assert!(config.s3.enable_auth);

        // 清理
        let _ = fs::remove_file(temp_file);
    }

    #[test]
    fn test_config_load_fallback() {
        // 测试当配置文件不存在时，使用默认配置
        let config = Config::load();
        assert_eq!(config.server.http_port, 8080);
    }

    #[test]
    fn test_config_clone() {
        let config = Config::default();
        let cloned = config.clone();

        assert_eq!(config.server.http_port, cloned.server.http_port);
        assert_eq!(config.storage.root_path, cloned.storage.root_path);
    }
}
