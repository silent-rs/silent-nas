//! WebDAV 增强上传处理器
//!
//! 集成内存监控、秒传、会话管理等高级功能

use super::handler::WebDavHandler;
use crate::models::{EventType, FileEvent};
use http::StatusCode;
use silent::prelude::*;
use silent_nas_core::StorageManagerTrait;
use std::time::Instant;

/// Body 读取器 (从 files.rs 复制)
struct BodyReader {
    body: ReqBody,
    buf: bytes::Bytes,
}

impl BodyReader {
    fn new(body: ReqBody) -> Self {
        Self {
            body,
            buf: bytes::Bytes::new(),
        }
    }
}

impl tokio::io::AsyncRead for BodyReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use futures_util::Stream;
        loop {
            if !self.buf.is_empty() {
                let to_copy = std::cmp::min(self.buf.len(), buf.remaining());
                let chunk = self.buf.split_to(to_copy);
                buf.put_slice(&chunk);
                return std::task::Poll::Ready(Ok(()));
            }

            match std::pin::Pin::new(&mut self.body).poll_next(cx) {
                std::task::Poll::Ready(Some(Ok(bytes))) => {
                    self.buf = bytes;
                    continue;
                }
                std::task::Poll::Ready(Some(Err(e))) => {
                    return std::task::Poll::Ready(Err(e));
                }
                std::task::Poll::Ready(None) => {
                    return std::task::Poll::Ready(Ok(()));
                }
                std::task::Poll::Pending => {
                    return std::task::Poll::Pending;
                }
            }
        }
    }
}

impl WebDavHandler {
    /// 增强版 PUT 处理器
    ///
    /// 支持:
    /// - 内存监控 (限制 100MB)
    /// - 秒传检查 (通过 X-File-Hash 头)
    /// - 会话管理 (通过 X-Upload-Session-Id 头)
    /// - 流式上传 (所有文件)
    ///
    /// # HTTP 头部
    /// - `X-File-Hash`: 文件 SHA-256 哈希 (用于秒传)
    /// - `X-Upload-Session-Id`: 上传会话 ID (用于续传)
    /// - `Content-Length`: 文件大小
    #[allow(dead_code)]
    pub(super) async fn handle_put_enhanced(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;

        // 获取请求头信息
        let content_length = req
            .headers()
            .get(http::header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let file_hash = req
            .headers()
            .get("X-File-Hash")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let session_id = req
            .headers()
            .get("X-Upload-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let storage_path = crate::storage::storage().get_full_path(&path);
        let file_exists = storage_path.exists();

        tracing::info!(
            "PUT Enhanced: path='{}' size={} hash={:?} session={:?}",
            path,
            content_length,
            file_hash.as_ref().map(|h| &h[..8]),
            session_id
        );

        // 1. 检查秒传
        #[allow(clippy::collapsible_if)]
        if let Some(ref hash) = file_hash {
            if let Some(existing_path) = self
                .instant_upload
                .check_instant_upload(hash, content_length)
                .await
            {
                tracing::info!(
                    "秒传成功: path='{}' existing='{}' hash={}",
                    path,
                    existing_path,
                    &hash[..8]
                );

                // 复制元数据
                let storage = crate::storage::storage();
                if let Ok(existing_metadata) = storage.get_metadata(&existing_path).await {
                    // 创建链接或复制元数据
                    let _ = storage.save_at_path(&path, &[]).await;

                    // 添加到秒传索引
                    self.instant_upload
                        .add_entry(hash.clone(), content_length, path.clone())
                        .await;

                    // 发布事件
                    let event_type = if file_exists {
                        EventType::Modified
                    } else {
                        EventType::Created
                    };
                    let mut event =
                        FileEvent::new(event_type, path.clone(), Some(existing_metadata));
                    event.source_http_addr = Some(self.source_http_addr.clone());

                    if let Some(ref n) = self.notifier {
                        if file_exists {
                            let _ = n.notify_modified(event).await;
                        } else {
                            let _ = n.notify_created(event).await;
                        }
                    }

                    let mut resp = Response::empty();
                    resp.set_status(if file_exists {
                        StatusCode::NO_CONTENT
                    } else {
                        StatusCode::CREATED
                    });
                    resp.headers_mut()
                        .insert("X-Instant-Upload", "true".parse().unwrap());

                    return Ok(resp);
                }
            }
        }

        // 2. 内存监控
        let chunk_size = 8 * 1024 * 1024; // 8MB
        if !self.memory_monitor.can_allocate(chunk_size) {
            tracing::warn!(
                "内存不足: 当前使用 {:.1}MB / {:.1}MB",
                self.memory_monitor.current_usage() as f64 / 1024.0 / 1024.0,
                self.memory_monitor.limit() as f64 / 1024.0 / 1024.0
            );
            return Err(SilentError::business_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "服务器内存不足，请稍后重试",
            ));
        }

        // 3. 创建或获取会话
        let session_id_for_cleanup = if let Some(sid) = session_id.clone() {
            // 续传：获取现有会话
            self.upload_sessions.get_session(&sid).await.map(|_| sid)
        } else {
            // 新上传：创建会话
            self.upload_sessions
                .create_session(path.clone(), content_length)
                .await
                .ok()
                .map(|s| s.session_id)
        };

        // 4. 流式上传
        let body = req.take_body();
        let upload_start = Instant::now();

        let size_desc = format_size(content_length);

        tracing::info!(
            "开始上传文件: path='{}' size={} session={:?}",
            path,
            size_desc,
            session_id_for_cleanup
        );

        match body {
            ReqBody::Incoming(incoming) => {
                let storage = crate::storage::storage();

                // 使用 BodyReader 进行流式读取
                let mut reader = BodyReader::new(ReqBody::Incoming(incoming));

                let save_start = Instant::now();
                let metadata = match storage.save_file_from_reader(&path, &mut reader).await {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(
                            "写入文件失败(流式): path='{}' size={} 耗时={:.2}s error={}",
                            path,
                            size_desc,
                            save_start.elapsed().as_secs_f64(),
                            e
                        );

                        // 标记会话失败
                        #[allow(clippy::collapsible_if)]
                        if let Some(ref sid) = session_id_for_cleanup {
                            if let Some(mut sess) = self.upload_sessions.get_session(sid).await {
                                sess.mark_failed();
                                let _ = self.upload_sessions.update_session(sess).await;
                            }
                        }

                        return Err(SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("写入文件失败: {}", e),
                        ));
                    }
                };

                tracing::info!(
                    "文件保存完成: path='{}' size={} 耗时={:.2}s",
                    path,
                    size_desc,
                    save_start.elapsed().as_secs_f64()
                );

                // 5. 更新秒传索引
                if let Some(hash) = file_hash.or_else(|| Some(metadata.hash.clone())) {
                    self.instant_upload
                        .add_entry(hash, metadata.size, path.clone())
                        .await;
                }

                // 6. 标记会话完成
                #[allow(clippy::collapsible_if)]
                if let Some(ref sid) = session_id_for_cleanup {
                    if let Some(mut sess) = self.upload_sessions.get_session(sid).await {
                        sess.mark_completed();
                        let _ = self.upload_sessions.update_session(sess).await;
                    }
                    // 异步清理会话（不等待）
                    let sessions_manager = self.upload_sessions.clone();
                    let sid_clone = sid.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
                        sessions_manager.remove_session(&sid_clone).await;
                    });
                }

                // 7. 发布事件
                let file_id = metadata.id.clone();
                let event_type = if file_exists {
                    EventType::Modified
                } else {
                    EventType::Created
                };
                let mut event = FileEvent::new(event_type, file_id, Some(metadata));
                event.source_http_addr = Some(self.source_http_addr.clone());

                if let Some(ref n) = self.notifier {
                    if file_exists {
                        let _ = n.notify_modified(event).await;
                    } else {
                        let _ = n.notify_created(event).await;
                    }
                }

                // 8. 记录变更
                if file_exists {
                    self.append_change("modified", &path);
                } else {
                    self.append_change("created", &path);
                }

                let mut resp = Response::empty();
                resp.set_status(if file_exists {
                    StatusCode::NO_CONTENT
                } else {
                    StatusCode::CREATED
                });

                // 添加性能指标头
                resp.headers_mut().insert(
                    "X-Upload-Time",
                    format!("{:.2}", upload_start.elapsed().as_secs_f64())
                        .parse()
                        .unwrap(),
                );

                tracing::info!(
                    "PUT Enhanced 完成: path='{}' status={} size={} 总耗时={:.2}s",
                    path,
                    if file_exists { 204 } else { 201 },
                    size_desc,
                    upload_start.elapsed().as_secs_f64()
                );

                Ok(resp)
            }
            ReqBody::Once(bytes) => {
                // 对于小文件，直接处理
                let body_data = bytes.to_vec();
                let size_desc = format_size(body_data.len() as u64);

                tracing::info!("开始保存文件(内存): path='{}' size={}", path, size_desc);

                let save_start = Instant::now();
                let metadata = crate::storage::storage()
                    .save_at_path(&path, &body_data)
                    .await
                    .map_err(|e| {
                        tracing::error!(
                            "写入文件失败: path='{}' size={} 耗时={:.2}s error={}",
                            path,
                            size_desc,
                            save_start.elapsed().as_secs_f64(),
                            e
                        );
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("写入文件失败: {}", e),
                        )
                    })?;

                // 更新秒传索引
                if let Some(hash) = file_hash.or_else(|| Some(metadata.hash.clone())) {
                    self.instant_upload
                        .add_entry(hash, metadata.size, path.clone())
                        .await;
                }

                // 标记会话完成
                #[allow(clippy::collapsible_if)]
                if let Some(ref sid) = session_id_for_cleanup {
                    if let Some(mut sess) = self.upload_sessions.get_session(sid).await {
                        sess.mark_completed();
                        let _ = self.upload_sessions.update_session(sess).await;
                    }
                }

                let file_id = metadata.id.clone();
                let event_type = if file_exists {
                    EventType::Modified
                } else {
                    EventType::Created
                };
                let mut event = FileEvent::new(event_type, file_id, Some(metadata));
                event.source_http_addr = Some(self.source_http_addr.clone());

                if let Some(ref n) = self.notifier {
                    if file_exists {
                        let _ = n.notify_modified(event).await;
                    } else {
                        let _ = n.notify_created(event).await;
                    }
                }

                if file_exists {
                    self.append_change("modified", &path);
                } else {
                    self.append_change("created", &path);
                }

                let mut resp = Response::empty();
                resp.set_status(if file_exists {
                    StatusCode::NO_CONTENT
                } else {
                    StatusCode::CREATED
                });

                tracing::info!(
                    "PUT Enhanced 完成: path='{}' status={} size={} 总耗时={:.2}s",
                    path,
                    if file_exists { 204 } else { 201 },
                    size_desc,
                    upload_start.elapsed().as_secs_f64()
                );

                Ok(resp)
            }
            ReqBody::Empty => Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "请求体为空",
            )),
        }
    }
}

/// 格式化文件大小
fn format_size(size: u64) -> String {
    if size >= 1024 * 1024 * 1024 {
        format!("{:.2}GB", size as f64 / 1024.0 / 1024.0 / 1024.0)
    } else if size >= 1024 * 1024 {
        format!("{:.2}MB", size as f64 / 1024.0 / 1024.0)
    } else if size >= 1024 {
        format!("{:.2}KB", size as f64 / 1024.0)
    } else {
        format!("{}B", size)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_size() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(100), "100B");
        assert_eq!(format_size(1024), "1.00KB");
        assert_eq!(format_size(1024 * 1024), "1.00MB");
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00GB");
        assert_eq!(format_size(1536 * 1024 * 1024), "1.50GB");
    }
}
