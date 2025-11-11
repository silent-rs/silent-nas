use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub server: ServerConfig,
    pub storage: StorageConfig,
    pub nats: NatsConfig,
    pub s3: S3Config,
    pub auth: AuthConfig,
    /// 节点发现/心跳配置
    #[serde(default)]
    pub node: NodeConfig,
    /// 跨节点同步行为配置
    #[serde(default)]
    pub sync: SyncBehaviorConfig,
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
    /// 存储引擎版本: "v1" (基础存储) 或 "v2" (增量存储，支持去重和压缩)
    #[serde(default = "StorageConfig::default_version")]
    pub version: String,
}

impl StorageConfig {
    fn default_version() -> String {
        "v1".to_string()
    }
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

/// 节点发现配置（对应 NodeDiscoveryConfig）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    /// 是否启用节点功能
    pub enable: bool,
    /// 种子节点地址列表（host:grpc_port）
    pub seed_nodes: Vec<String>,
    /// 心跳间隔（秒）
    pub heartbeat_interval: u64,
    /// 节点超时（秒）
    pub node_timeout: i64,
}

impl Default for NodeConfig {
    fn default() -> Self {
        Self {
            enable: true,
            seed_nodes: Vec::new(),
            heartbeat_interval: 10,
            node_timeout: 30,
        }
    }
}

/// 跨节点同步行为配置（对应 SyncConfig）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncBehaviorConfig {
    /// 是否自动同步
    pub auto_sync: bool,
    /// 同步间隔（秒）
    pub sync_interval: u64,
    /// 每次同步的最大文件数
    pub max_files_per_sync: usize,
    /// 同步并发文件数
    #[serde(default = "SyncBehaviorConfig::default_max_concurrency")]
    pub max_concurrency: usize,
    /// 失败重试次数
    pub max_retries: u32,
    /// 拉取连接超时（秒）
    #[serde(default = "SyncBehaviorConfig::default_http_connect_timeout")]
    pub http_connect_timeout: u64,
    /// 拉取请求超时（秒）
    #[serde(default = "SyncBehaviorConfig::default_http_request_timeout")]
    pub http_request_timeout: u64,
    /// 拉取最大重试次数
    #[serde(default = "SyncBehaviorConfig::default_fetch_max_retries")]
    pub fetch_max_retries: u32,
    /// 拉取退避基数（秒）
    #[serde(default = "SyncBehaviorConfig::default_fetch_base_backoff")]
    pub fetch_base_backoff: u64,
    /// 拉取退避上限（秒）
    #[serde(default = "SyncBehaviorConfig::default_fetch_max_backoff")]
    pub fetch_max_backoff: u64,
    /// 失败补偿队列容量上限
    #[serde(default = "SyncBehaviorConfig::default_fail_queue_max")]
    pub fail_queue_max: usize,
    /// 失败任务TTL（秒），超过即丢弃
    #[serde(default = "SyncBehaviorConfig::default_fail_task_ttl_secs")]
    pub fail_task_ttl_secs: u64,
    /// gRPC 连接超时（秒）
    #[serde(default = "SyncBehaviorConfig::default_grpc_connect_timeout")]
    pub grpc_connect_timeout: u64,
    /// gRPC 请求超时（秒）
    #[serde(default = "SyncBehaviorConfig::default_grpc_request_timeout")]
    pub grpc_request_timeout: u64,
    /// 故障注入：传输失败概率（0.0-1.0）
    #[serde(default = "SyncBehaviorConfig::default_fault_transfer_rate")]
    pub fault_transfer_error_rate: f64,
    /// 故障注入：校验失败概率（0.0-1.0）
    #[serde(default = "SyncBehaviorConfig::default_fault_verify_rate")]
    pub fault_verify_error_rate: f64,
    /// 故障注入：额外延迟（毫秒）
    #[serde(default = "SyncBehaviorConfig::default_fault_delay_ms")]
    pub fault_delay_ms: u64,
}

impl Default for SyncBehaviorConfig {
    fn default() -> Self {
        Self {
            auto_sync: true,
            sync_interval: 60,
            max_files_per_sync: 100,
            max_concurrency: Self::default_max_concurrency(),
            max_retries: 3,
            http_connect_timeout: Self::default_http_connect_timeout(),
            http_request_timeout: Self::default_http_request_timeout(),
            fetch_max_retries: Self::default_fetch_max_retries(),
            fetch_base_backoff: Self::default_fetch_base_backoff(),
            fetch_max_backoff: Self::default_fetch_max_backoff(),
            fail_queue_max: Self::default_fail_queue_max(),
            fail_task_ttl_secs: Self::default_fail_task_ttl_secs(),
            grpc_connect_timeout: Self::default_grpc_connect_timeout(),
            grpc_request_timeout: Self::default_grpc_request_timeout(),
            fault_transfer_error_rate: Self::default_fault_transfer_rate(),
            fault_verify_error_rate: Self::default_fault_verify_rate(),
            fault_delay_ms: Self::default_fault_delay_ms(),
        }
    }
}

impl SyncBehaviorConfig {
    fn default_max_concurrency() -> usize {
        8
    }
    fn default_http_connect_timeout() -> u64 {
        5
    }
    fn default_http_request_timeout() -> u64 {
        15
    }
    fn default_fetch_max_retries() -> u32 {
        3
    }
    fn default_fetch_base_backoff() -> u64 {
        1
    }
    fn default_fetch_max_backoff() -> u64 {
        8
    }
    fn default_fail_queue_max() -> usize {
        1000
    }
    fn default_fail_task_ttl_secs() -> u64 {
        24 * 3600
    }
    fn default_grpc_connect_timeout() -> u64 {
        10
    }
    fn default_grpc_request_timeout() -> u64 {
        30
    }
    fn default_fault_transfer_rate() -> f64 {
        0.0
    }
    fn default_fault_verify_rate() -> f64 {
        0.0
    }
    fn default_fault_delay_ms() -> u64 {
        0
    }
}

/// 认证配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthConfig {
    /// 是否启用认证
    pub enable: bool,
    /// 数据库路径
    pub db_path: String,
    /// JWT密钥
    pub jwt_secret: String,
    /// 访问令牌过期时间（秒）
    pub access_token_exp: u64,
    /// 刷新令牌过期时间（秒）
    pub refresh_token_exp: u64,
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
                version: "v1".to_string(),
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
            node: NodeConfig {
                enable: true,
                seed_nodes: Vec::new(),
                heartbeat_interval: 10,
                node_timeout: 30,
            },
            sync: SyncBehaviorConfig {
                auto_sync: true,
                sync_interval: 60,
                max_files_per_sync: 100,
                max_concurrency: SyncBehaviorConfig::default_max_concurrency(),
                max_retries: 3,
                http_connect_timeout: SyncBehaviorConfig::default_http_connect_timeout(),
                http_request_timeout: SyncBehaviorConfig::default_http_request_timeout(),
                fetch_max_retries: SyncBehaviorConfig::default_fetch_max_retries(),
                fetch_base_backoff: SyncBehaviorConfig::default_fetch_base_backoff(),
                fetch_max_backoff: SyncBehaviorConfig::default_fetch_max_backoff(),
                fail_queue_max: SyncBehaviorConfig::default_fail_queue_max(),
                fail_task_ttl_secs: SyncBehaviorConfig::default_fail_task_ttl_secs(),
                grpc_connect_timeout: SyncBehaviorConfig::default_grpc_connect_timeout(),
                grpc_request_timeout: SyncBehaviorConfig::default_grpc_request_timeout(),
                fault_transfer_error_rate: SyncBehaviorConfig::default_fault_transfer_rate(),
                fault_verify_error_rate: SyncBehaviorConfig::default_fault_verify_rate(),
                fault_delay_ms: SyncBehaviorConfig::default_fault_delay_ms(),
            },
            auth: AuthConfig {
                enable: false,
                db_path: "./data/auth.db".to_string(),
                jwt_secret: "silent-nas-secret-key-change-in-production".to_string(),
                access_token_exp: 3600,    // 1小时
                refresh_token_exp: 604800, // 7天
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
        let mut config = Self::from_file("config.toml").unwrap_or_default();
        config.apply_env_overrides();
        config
    }

    /// 应用环境变量覆盖配置
    pub fn apply_env_overrides(&mut self) {
        // 认证配置
        if let Ok(enable) = std::env::var("ENABLE_AUTH") {
            self.auth.enable = enable.to_lowercase() == "true" || enable == "1";
        }
        if let Ok(db_path) = std::env::var("AUTH_DB_PATH") {
            self.auth.db_path = db_path;
        }
        if let Ok(jwt_secret) = std::env::var("JWT_SECRET") {
            self.auth.jwt_secret = jwt_secret;
        }
        if let Ok(exp) = std::env::var("JWT_ACCESS_EXP")
            && let Ok(seconds) = exp.parse::<u64>()
        {
            self.auth.access_token_exp = seconds;
        }
        if let Ok(exp) = std::env::var("JWT_REFRESH_EXP")
            && let Ok(seconds) = exp.parse::<u64>()
        {
            self.auth.refresh_token_exp = seconds;
        }

        // 节点与同步配置（可选）
        if let Ok(enable_node) = std::env::var("NODE_ENABLE") {
            self.node.enable = enable_node.to_lowercase() == "true" || enable_node == "1";
        }
        if let Ok(seeds) = std::env::var("NODE_SEEDS") {
            // 以逗号分隔的种子节点列表
            self.node.seed_nodes = seeds
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Ok(hb) = std::env::var("NODE_HEARTBEAT")
            && let Ok(v) = hb.parse::<u64>()
        {
            self.node.heartbeat_interval = v;
        }
        if let Ok(nt) = std::env::var("NODE_TIMEOUT")
            && let Ok(v) = nt.parse::<i64>()
        {
            self.node.node_timeout = v;
        }

        if let Ok(auto) = std::env::var("SYNC_AUTO") {
            self.sync.auto_sync = auto.to_lowercase() == "true" || auto == "1";
        }
        if let Ok(si) = std::env::var("SYNC_INTERVAL")
            && let Ok(v) = si.parse::<u64>()
        {
            self.sync.sync_interval = v;
        }
        if let Ok(mfps) = std::env::var("SYNC_MAX_FILES")
            && let Ok(v) = mfps.parse::<usize>()
        {
            self.sync.max_files_per_sync = v;
        }
        if let Ok(mc) = std::env::var("SYNC_MAX_CONCURRENCY")
            && let Ok(v) = mc.parse::<usize>()
        {
            self.sync.max_concurrency = v;
        }
        if let Ok(retry) = std::env::var("SYNC_MAX_RETRIES")
            && let Ok(v) = retry.parse::<u32>()
        {
            self.sync.max_retries = v;
        }
        // 可选：覆盖拉取超时与退避配置（仍优先以配置文件驱动）
        if let Ok(v) = std::env::var("SYNC_HTTP_CONNECT_TIMEOUT")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.http_connect_timeout = n;
        }
        if let Ok(v) = std::env::var("SYNC_HTTP_REQUEST_TIMEOUT")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.http_request_timeout = n;
        }
        if let Ok(v) = std::env::var("SYNC_FETCH_MAX_RETRIES")
            && let Ok(n) = v.parse::<u32>()
        {
            self.sync.fetch_max_retries = n;
        }
        if let Ok(v) = std::env::var("SYNC_FETCH_BASE_BACKOFF")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.fetch_base_backoff = n;
        }
        if let Ok(v) = std::env::var("SYNC_FETCH_MAX_BACKOFF")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.fetch_max_backoff = n;
        }

        if let Ok(v) = std::env::var("SYNC_FAIL_QUEUE_MAX")
            && let Ok(n) = v.parse::<usize>()
        {
            self.sync.fail_queue_max = n;
        }
        if let Ok(v) = std::env::var("SYNC_FAIL_TASK_TTL")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.fail_task_ttl_secs = n;
        }
        if let Ok(v) = std::env::var("SYNC_GRPC_CONNECT_TIMEOUT")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.grpc_connect_timeout = n;
        }
        if let Ok(v) = std::env::var("SYNC_GRPC_REQUEST_TIMEOUT")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.grpc_request_timeout = n;
        }
        if let Ok(v) = std::env::var("SYNC_FAULT_TRANSFER_RATE")
            && let Ok(n) = v.parse::<f64>()
        {
            self.sync.fault_transfer_error_rate = n.clamp(0.0, 1.0);
        }
        if let Ok(v) = std::env::var("SYNC_FAULT_VERIFY_RATE")
            && let Ok(n) = v.parse::<f64>()
        {
            self.sync.fault_verify_error_rate = n.clamp(0.0, 1.0);
        }
        if let Ok(v) = std::env::var("SYNC_FAULT_DELAY_MS")
            && let Ok(n) = v.parse::<u64>()
        {
            self.sync.fault_delay_ms = n;
        }
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

        // 测试认证配置
        assert!(!config.auth.enable);
        assert_eq!(config.auth.db_path, "./data/auth.db");
        assert_eq!(
            config.auth.jwt_secret,
            "silent-nas-secret-key-change-in-production"
        );
        assert_eq!(config.auth.access_token_exp, 3600);
        assert_eq!(config.auth.refresh_token_exp, 604800);
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
            version: "v2".to_string(),
        };

        assert_eq!(storage.root_path, PathBuf::from("/tmp/storage"));
        assert_eq!(storage.chunk_size, 8 * 1024 * 1024);
        assert_eq!(storage.version, "v2");
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

[auth]
enable = true
db_path = "/tmp/auth.db"
jwt_secret = "test-secret"
access_token_exp = 7200
refresh_token_exp = 1209600
"#;
        fs::write(temp_file, config_content).unwrap();

        let config = Config::from_file(temp_file).unwrap();
        assert_eq!(config.server.http_port, 9999);
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.storage.chunk_size, 8388608);
        assert_eq!(config.nats.url, "nats://test:4222");
        assert_eq!(config.s3.access_key, "testkey");
        assert!(config.s3.enable_auth);
        assert!(config.auth.enable);
        assert_eq!(config.auth.db_path, "/tmp/auth.db");

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

    #[test]
    fn test_auth_config() {
        let auth = AuthConfig {
            enable: true,
            db_path: "/tmp/auth.db".to_string(),
            jwt_secret: "test-secret".to_string(),
            access_token_exp: 7200,
            refresh_token_exp: 1209600,
        };

        assert!(auth.enable);
        assert_eq!(auth.db_path, "/tmp/auth.db");
        assert_eq!(auth.jwt_secret, "test-secret");
        assert_eq!(auth.access_token_exp, 7200);
        assert_eq!(auth.refresh_token_exp, 1209600);
    }

    #[test]
    fn test_apply_env_overrides() {
        // 设置环境变量
        unsafe {
            std::env::set_var("ENABLE_AUTH", "true");
            std::env::set_var("AUTH_DB_PATH", "/custom/auth.db");
            std::env::set_var("JWT_SECRET", "custom-secret");
            std::env::set_var("JWT_ACCESS_EXP", "7200");
            std::env::set_var("JWT_REFRESH_EXP", "1209600");
        }

        let mut config = Config::default();
        config.apply_env_overrides();

        assert!(config.auth.enable);
        assert_eq!(config.auth.db_path, "/custom/auth.db");
        assert_eq!(config.auth.jwt_secret, "custom-secret");
        assert_eq!(config.auth.access_token_exp, 7200);
        assert_eq!(config.auth.refresh_token_exp, 1209600);

        // 清理环境变量
        unsafe {
            std::env::remove_var("ENABLE_AUTH");
            std::env::remove_var("AUTH_DB_PATH");
            std::env::remove_var("JWT_SECRET");
            std::env::remove_var("JWT_ACCESS_EXP");
            std::env::remove_var("JWT_REFRESH_EXP");
        }
    }

    #[test]
    fn test_config_with_auth_section() {
        // 创建临时配置文件
        let temp_file = "./test_auth_config.toml";
        let config_content = r#"
[server]
http_port = 8080
grpc_port = 50051
quic_port = 4433
webdav_port = 8081
s3_port = 9000
host = "127.0.0.1"

[storage]
root_path = "./storage"
chunk_size = 4194304

[nats]
url = "nats://127.0.0.1:4222"
topic_prefix = "silent.nas.files"

[s3]
access_key = "minioadmin"
secret_key = "minioadmin"
enable_auth = false

[auth]
enable = true
db_path = "/var/lib/auth.db"
jwt_secret = "production-secret"
access_token_exp = 7200
refresh_token_exp = 1209600
"#;
        fs::write(temp_file, config_content).unwrap();

        let config = Config::from_file(temp_file).unwrap();
        assert!(config.auth.enable);
        assert_eq!(config.auth.db_path, "/var/lib/auth.db");
        assert_eq!(config.auth.jwt_secret, "production-secret");
        assert_eq!(config.auth.access_token_exp, 7200);
        assert_eq!(config.auth.refresh_token_exp, 1209600);

        // 清理
        let _ = fs::remove_file(temp_file);
    }
}
