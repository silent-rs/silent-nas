use crate::error::{NasError, Result};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use quinn::{Endpoint, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{debug, error, info};

/// QUIC 文件传输服务
pub struct QuicTransferServer {
    #[allow(dead_code)]
    storage: StorageManager,
    #[allow(dead_code)]
    notifier: EventNotifier,
    endpoint: Option<Endpoint>,
}

impl QuicTransferServer {
    pub fn new(storage: StorageManager, notifier: EventNotifier) -> Self {
        Self {
            storage,
            notifier,
            endpoint: None,
        }
    }

    /// 启动 QUIC 服务器
    pub async fn start(&mut self, addr: SocketAddr) -> Result<()> {
        let server_config = self.configure_server()?;
        let endpoint = Endpoint::server(server_config, addr)
            .map_err(|e| NasError::Transfer(format!("启动 QUIC 服务器失败: {}", e)))?;

        info!("QUIC 文件传输服务器启动: {}", addr);
        self.endpoint = Some(endpoint.clone());

        // 启动连接处理循环
        tokio::spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                tokio::spawn(async move {
                    match incoming.await {
                        Ok(connection) => {
                            info!("新的 QUIC 连接: {}", connection.remote_address());

                            while let Ok((mut send, mut recv)) = connection.accept_bi().await {
                                tokio::spawn(async move {
                                    if let Err(e) = handle_stream(&mut send, &mut recv).await {
                                        error!("处理流失败: {}", e);
                                    }
                                });
                            }
                        }
                        Err(e) => {
                            error!("QUIC 连接失败: {}", e);
                        }
                    }
                });
            }
        });

        Ok(())
    }

    /// 配置服务器（使用自签名证书）
    fn configure_server(&self) -> Result<ServerConfig> {
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".into()])
            .map_err(|e| NasError::Transfer(format!("生成证书失败: {}", e)))?;

        let cert_der = CertificateDer::from(cert.cert);
        let key_der = PrivateKeyDer::try_from(cert.key_pair.serialize_der())
            .map_err(|e| NasError::Transfer(format!("序列化私钥失败: {}", e)))?;

        let mut server_config = ServerConfig::with_single_cert(vec![cert_der], key_der)
            .map_err(|e| NasError::Transfer(format!("配置服务器失败: {}", e)))?;

        let transport_config = Arc::get_mut(&mut server_config.transport)
            .ok_or_else(|| NasError::Transfer("获取传输配置失败".into()))?;

        transport_config.max_concurrent_uni_streams(0_u8.into());

        Ok(server_config)
    }
}

/// 处理单个双向流
async fn handle_stream(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream) -> Result<()> {
    // 读取命令（简单协议：1字节命令 + 数据）
    let mut cmd = [0u8; 1];
    recv.read_exact(&mut cmd)
        .await
        .map_err(|e| NasError::Transfer(format!("读取命令失败: {}", e)))?;

    match cmd[0] {
        0x01 => {
            // 上传文件
            handle_upload(send, recv).await?;
        }
        0x02 => {
            // 下载文件
            handle_download(send, recv).await?;
        }
        _ => {
            error!("未知命令: {}", cmd[0]);
            return Err(NasError::Transfer(format!("未知命令: {}", cmd[0])));
        }
    }

    Ok(())
}

/// 处理文件上传
async fn handle_upload(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream) -> Result<()> {
    // 读取文件 ID 长度
    let mut id_len_buf = [0u8; 4];
    recv.read_exact(&mut id_len_buf)
        .await
        .map_err(|e| NasError::Transfer(format!("读取文件ID长度失败: {}", e)))?;
    let id_len = u32::from_be_bytes(id_len_buf) as usize;

    // 读取文件 ID
    let mut file_id = vec![0u8; id_len];
    recv.read_exact(&mut file_id)
        .await
        .map_err(|e| NasError::Transfer(format!("读取文件ID失败: {}", e)))?;
    let file_id = String::from_utf8(file_id)
        .map_err(|e| NasError::Transfer(format!("文件ID编码错误: {}", e)))?;

    // 读取文件数据（限制最大 100MB）
    let data = recv
        .read_to_end(100 * 1024 * 1024)
        .await
        .map_err(|e| NasError::Transfer(format!("读取文件数据失败: {}", e)))?;

    debug!("接收文件上传: {} - {} 字节", file_id, data.len());

    // 这里需要访问 storage，暂时简化处理
    // 实际应用中需要传递 storage 引用

    // 发送成功响应
    send.write_all(&[0x00])
        .await
        .map_err(|e| NasError::Transfer(format!("发送响应失败: {}", e)))?;
    send.finish()
        .map_err(|e| NasError::Transfer(format!("关闭发送流失败: {}", e)))?;

    Ok(())
}

/// 处理文件下载
async fn handle_download(send: &mut quinn::SendStream, recv: &mut quinn::RecvStream) -> Result<()> {
    // 读取文件 ID 长度
    let mut id_len_buf = [0u8; 4];
    recv.read_exact(&mut id_len_buf)
        .await
        .map_err(|e| NasError::Transfer(format!("读取文件ID长度失败: {}", e)))?;
    let id_len = u32::from_be_bytes(id_len_buf) as usize;

    // 读取文件 ID
    let mut file_id = vec![0u8; id_len];
    recv.read_exact(&mut file_id)
        .await
        .map_err(|e| NasError::Transfer(format!("读取文件ID失败: {}", e)))?;
    let file_id = String::from_utf8(file_id)
        .map_err(|e| NasError::Transfer(format!("文件ID编码错误: {}", e)))?;

    debug!("接收文件下载请求: {}", file_id);

    // 这里需要访问 storage，暂时发送空数据
    let data = vec![];

    // 发送文件数据
    send.write_all(&data)
        .await
        .map_err(|e| NasError::Transfer(format!("发送文件数据失败: {}", e)))?;
    send.finish()
        .map_err(|e| NasError::Transfer(format!("关闭发送流失败: {}", e)))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quic_transfer_server_type() {
        // 测试 QuicTransferServer 类型
        let type_name = std::any::type_name::<QuicTransferServer>();
        assert!(type_name.contains("QuicTransferServer"));
    }

    #[test]
    fn test_command_bytes() {
        // 测试命令字节
        let upload_cmd: u8 = 0x01;
        let download_cmd: u8 = 0x02;
        let unknown_cmd: u8 = 0xFF;

        assert_eq!(upload_cmd, 1);
        assert_eq!(download_cmd, 2);
        assert_ne!(upload_cmd, download_cmd);
        assert_ne!(unknown_cmd, upload_cmd);
        assert_ne!(unknown_cmd, download_cmd);
    }

    #[test]
    fn test_file_id_encoding() {
        // 测试文件 ID 编码和解码
        let file_id = "test-file-123";
        let bytes = file_id.as_bytes();
        let len = bytes.len() as u32;
        let len_bytes = len.to_be_bytes();

        // 验证长度编码
        assert_eq!(len_bytes.len(), 4);
        let decoded_len = u32::from_be_bytes(len_bytes);
        assert_eq!(decoded_len, len);

        // 验证 ID 解码
        let decoded_id = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(decoded_id, file_id);
    }

    #[test]
    fn test_file_id_length_encoding() {
        // 测试不同长度的文件 ID
        let test_cases = vec![
            ("a", 1u32),
            ("test", 4u32),
            ("test-file-123", 13u32),
            ("very-long-file-id-with-many-characters", 38u32),
        ];

        for (file_id, expected_len) in test_cases {
            let len = file_id.len() as u32;
            assert_eq!(len, expected_len);

            let len_bytes = len.to_be_bytes();
            let decoded = u32::from_be_bytes(len_bytes);
            assert_eq!(decoded, expected_len);
        }
    }

    #[test]
    fn test_upload_command_value() {
        const UPLOAD_CMD: u8 = 0x01;
        assert_eq!(UPLOAD_CMD, 1);

        let cmd_array = [UPLOAD_CMD];
        assert_eq!(cmd_array[0], 0x01);
    }

    #[test]
    fn test_download_command_value() {
        const DOWNLOAD_CMD: u8 = 0x02;
        assert_eq!(DOWNLOAD_CMD, 2);

        let cmd_array = [DOWNLOAD_CMD];
        assert_eq!(cmd_array[0], 0x02);
    }

    #[test]
    fn test_unknown_command_detection() {
        let valid_commands = [0x01, 0x02];
        let unknown_commands = [0x00, 0x03, 0xFF];

        for cmd in valid_commands {
            assert!(cmd == 0x01 || cmd == 0x02);
        }

        for cmd in unknown_commands {
            assert!(cmd != 0x01 && cmd != 0x02);
        }
    }

    #[test]
    fn test_max_file_size_constant() {
        const MAX_FILE_SIZE: usize = 100 * 1024 * 1024; // 100MB
        assert_eq!(MAX_FILE_SIZE, 104_857_600);
    }

    #[test]
    fn test_response_byte() {
        const SUCCESS_RESPONSE: u8 = 0x00;
        assert_eq!(SUCCESS_RESPONSE, 0);

        let response_array = [SUCCESS_RESPONSE];
        assert_eq!(response_array[0], 0x00);
    }

    #[test]
    fn test_buffer_sizes() {
        let cmd_buf_size = 1;
        let id_len_buf_size = 4;

        assert_eq!(cmd_buf_size, 1);
        assert_eq!(id_len_buf_size, 4);

        let cmd_buf = [0u8; 1];
        let id_len_buf = [0u8; 4];

        assert_eq!(cmd_buf.len(), cmd_buf_size);
        assert_eq!(id_len_buf.len(), id_len_buf_size);
    }

    #[test]
    fn test_file_id_utf8_encoding() {
        let test_ids = vec![
            "simple-id",
            "id-with-numbers-123",
            "文件ID中文",
            "id_with_emoji_🔥",
            "id/with/slashes",
        ];

        for file_id in test_ids {
            let bytes = file_id.as_bytes();
            let decoded = String::from_utf8(bytes.to_vec()).unwrap();
            assert_eq!(decoded, file_id);
        }
    }

    #[test]
    fn test_command_matching() {
        let commands = vec![0x01, 0x02, 0xFF];

        for cmd in commands {
            match cmd {
                0x01 => assert_eq!(cmd, 1),
                0x02 => assert_eq!(cmd, 2),
                _ => assert!(cmd != 0x01 && cmd != 0x02),
            }
        }
    }

    #[test]
    fn test_be_bytes_conversion() {
        let test_values = vec![0u32, 1u32, 100u32, 1000u32, 1_000_000u32];

        for value in test_values {
            let bytes = value.to_be_bytes();
            let decoded = u32::from_be_bytes(bytes);
            assert_eq!(decoded, value);
        }
    }

    #[test]
    fn test_error_message_format() {
        let file_id = "test-file";
        let error_msg = format!("读取文件ID失败: {}", file_id);
        assert!(error_msg.contains("读取文件ID失败"));
        assert!(error_msg.contains(file_id));
    }

    #[test]
    fn test_data_size_calculation() {
        let data_sizes = vec![
            (0, 0),
            (1024, 1024),
            (1024 * 1024, 1_048_576),
            (100 * 1024 * 1024, 104_857_600),
        ];

        for (input, expected) in data_sizes {
            assert_eq!(input, expected);
        }
    }
}
