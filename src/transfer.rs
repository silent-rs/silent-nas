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
