//! WebDAV 大文件上传会话管理
//!
//! 支持:
//! - 断点续传
//! - 秒传
//! - 临时文件管理
//! - 内存占用监控

use chrono::NaiveDateTime;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 上传会话状态
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub enum UploadStatus {
    /// 初始化中
    Initializing,
    /// 上传中
    Uploading,
    /// 已暂停 (可续传)
    Paused,
    /// 已完成
    Completed,
    /// 失败
    Failed,
    /// 已取消
    Cancelled,
}

/// 上传会话信息
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)]
pub struct UploadSession {
    /// 会话ID
    pub session_id: String,
    /// 目标文件路径
    pub file_path: String,
    /// 临时文件路径
    pub temp_path: Option<PathBuf>,
    /// 文件总大小 (字节)
    pub total_size: u64,
    /// 已上传大小 (字节)
    pub uploaded_size: u64,
    /// 文件哈希 (可选，用于秒传)
    pub file_hash: Option<String>,
    /// 状态
    pub status: UploadStatus,
    /// 创建时间
    pub created_at: NaiveDateTime,
    /// 最后更新时间
    pub updated_at: NaiveDateTime,
    /// 过期时间
    pub expires_at: NaiveDateTime,
    /// 已上传的块ID列表 (用于断点续传)
    pub uploaded_chunks: Vec<String>,
    /// 内存使用量 (字节)
    pub memory_usage: u64,
}

impl UploadSession {
    /// 创建新的上传会话
    #[allow(dead_code)]
    pub fn new(file_path: String, total_size: u64, ttl_hours: i64) -> Self {
        let now = chrono::Local::now().naive_local();
        let session_id = format!("upload_{}", scru128::new_string());

        Self {
            session_id,
            file_path,
            temp_path: None,
            total_size,
            uploaded_size: 0,
            file_hash: None,
            status: UploadStatus::Initializing,
            created_at: now,
            updated_at: now,
            expires_at: now + chrono::Duration::hours(ttl_hours),
            uploaded_chunks: Vec::new(),
            memory_usage: 0,
        }
    }

    /// 检查会话是否过期
    pub fn is_expired(&self) -> bool {
        let now = chrono::Local::now().naive_local();
        now > self.expires_at
    }

    /// 计算上传进度百分比
    #[allow(dead_code)]
    pub fn progress_percent(&self) -> f64 {
        if self.total_size == 0 {
            0.0
        } else {
            (self.uploaded_size as f64 / self.total_size as f64) * 100.0
        }
    }

    /// 检查是否可以续传
    #[allow(dead_code)]
    pub fn can_resume(&self) -> bool {
        matches!(self.status, UploadStatus::Paused | UploadStatus::Failed)
            && !self.is_expired()
            && self.uploaded_size < self.total_size
    }

    /// 更新上传进度
    #[allow(dead_code)]
    pub fn update_progress(&mut self, uploaded_size: u64) {
        self.uploaded_size = uploaded_size;
        self.updated_at = chrono::Local::now().naive_local();
    }

    /// 添加已上传的块
    #[allow(dead_code)]
    pub fn add_uploaded_chunk(&mut self, chunk_id: String) {
        self.uploaded_chunks.push(chunk_id);
        self.updated_at = chrono::Local::now().naive_local();
    }

    /// 标记为完成
    #[allow(dead_code)]
    pub fn mark_completed(&mut self) {
        self.status = UploadStatus::Completed;
        self.uploaded_size = self.total_size;
        self.updated_at = chrono::Local::now().naive_local();
    }

    /// 标记为失败
    #[allow(dead_code)]
    pub fn mark_failed(&mut self) {
        self.status = UploadStatus::Failed;
        self.updated_at = chrono::Local::now().naive_local();
    }
}

/// 上传会话管理器
#[allow(dead_code)]
pub struct UploadSessionManager {
    /// 活跃的上传会话 (session_id -> UploadSession)
    sessions: Arc<RwLock<HashMap<String, UploadSession>>>,
    /// 临时文件目录
    temp_dir: PathBuf,
    /// 会话默认过期时间 (小时)
    default_ttl_hours: i64,
    /// 最大并发上传数
    max_concurrent_uploads: usize,
}

impl UploadSessionManager {
    /// 创建新的会话管理器
    #[allow(dead_code)]
    pub fn new(temp_dir: PathBuf, default_ttl_hours: i64, max_concurrent_uploads: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            temp_dir,
            default_ttl_hours,
            max_concurrent_uploads,
        }
    }

    /// 创建新的上传会话
    #[allow(dead_code)]
    pub async fn create_session(
        &self,
        file_path: String,
        total_size: u64,
    ) -> Result<UploadSession, String> {
        // 检查并发上传限制
        let sessions = self.sessions.read().await;
        let active_count = sessions
            .values()
            .filter(|s| s.status == UploadStatus::Uploading)
            .count();

        if active_count >= self.max_concurrent_uploads {
            return Err(format!(
                "超过最大并发上传数限制: {}/{}",
                active_count, self.max_concurrent_uploads
            ));
        }
        drop(sessions);

        // 创建新会话
        let session = UploadSession::new(file_path, total_size, self.default_ttl_hours);
        let session_id = session.session_id.clone();

        // 保存会话
        let mut sessions = self.sessions.write().await;
        sessions.insert(session_id, session.clone());

        Ok(session)
    }

    /// 获取会话
    #[allow(dead_code)]
    pub async fn get_session(&self, session_id: &str) -> Option<UploadSession> {
        let sessions = self.sessions.read().await;
        sessions.get(session_id).cloned()
    }

    /// 更新会话
    #[allow(dead_code)]
    pub async fn update_session(&self, session: UploadSession) -> Result<(), String> {
        let mut sessions = self.sessions.write().await;
        if !sessions.contains_key(&session.session_id) {
            return Err(format!("会话不存在: {}", session.session_id));
        }
        sessions.insert(session.session_id.clone(), session);
        Ok(())
    }

    /// 删除会话
    #[allow(dead_code)]
    pub async fn remove_session(&self, session_id: &str) -> Option<UploadSession> {
        let mut sessions = self.sessions.write().await;
        sessions.remove(session_id)
    }

    /// 清理过期会话
    #[allow(dead_code)]
    pub async fn cleanup_expired_sessions(&self) -> usize {
        let mut sessions = self.sessions.write().await;
        let expired_ids: Vec<String> = sessions
            .iter()
            .filter(|(_, s)| s.is_expired())
            .map(|(id, _)| id.clone())
            .collect();

        let count = expired_ids.len();
        for id in expired_ids {
            if let Some(session) = sessions.remove(&id) {
                // 清理临时文件
                if let Some(temp_path) = session.temp_path {
                    let _ = tokio::fs::remove_file(&temp_path).await;
                }
            }
        }

        count
    }

    /// 创建临时文件路径
    #[allow(dead_code)]
    pub fn create_temp_path(&self, session_id: &str) -> PathBuf {
        self.temp_dir.join(format!("{}.tmp", session_id))
    }

    /// 获取所有活跃会话
    #[allow(dead_code)]
    pub async fn get_active_sessions(&self) -> Vec<UploadSession> {
        let sessions = self.sessions.read().await;
        sessions
            .values()
            .filter(|s| s.status == UploadStatus::Uploading && !s.is_expired())
            .cloned()
            .collect()
    }

    /// 获取会话总内存使用量
    #[allow(dead_code)]
    pub async fn total_memory_usage(&self) -> u64 {
        let sessions = self.sessions.read().await;
        sessions.values().map(|s| s.memory_usage).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_upload_session_new() {
        let session = UploadSession::new("/test/file.txt".to_string(), 1000, 24);
        assert_eq!(session.file_path, "/test/file.txt");
        assert_eq!(session.total_size, 1000);
        assert_eq!(session.uploaded_size, 0);
        assert_eq!(session.status, UploadStatus::Initializing);
        assert!(!session.is_expired());
        assert_eq!(session.progress_percent(), 0.0);
    }

    #[test]
    fn test_upload_session_progress() {
        let mut session = UploadSession::new("/test/file.txt".to_string(), 1000, 24);
        session.update_progress(500);
        assert_eq!(session.uploaded_size, 500);
        assert_eq!(session.progress_percent(), 50.0);

        session.update_progress(1000);
        assert_eq!(session.uploaded_size, 1000);
        assert_eq!(session.progress_percent(), 100.0);
    }

    #[test]
    fn test_upload_session_can_resume() {
        let mut session = UploadSession::new("/test/file.txt".to_string(), 1000, 24);

        // 初始化状态不能续传
        assert!(!session.can_resume());

        // 暂停状态可以续传
        session.status = UploadStatus::Paused;
        session.uploaded_size = 500;
        assert!(session.can_resume());

        // 失败状态可以续传
        session.status = UploadStatus::Failed;
        assert!(session.can_resume());

        // 已完成不能续传
        session.status = UploadStatus::Completed;
        session.uploaded_size = 1000;
        assert!(!session.can_resume());
    }

    #[tokio::test]
    async fn test_session_manager_create() {
        let temp_dir = std::env::temp_dir().join("webdav_upload_test");
        let manager = UploadSessionManager::new(temp_dir, 24, 10);

        let session = manager
            .create_session("/test/file.txt".to_string(), 1000)
            .await
            .unwrap();

        assert_eq!(session.file_path, "/test/file.txt");
        assert_eq!(session.total_size, 1000);

        // 验证可以获取会话
        let retrieved = manager.get_session(&session.session_id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().session_id, session.session_id);
    }

    #[tokio::test]
    async fn test_session_manager_concurrent_limit() {
        let temp_dir = std::env::temp_dir().join("webdav_upload_test2");
        let manager = UploadSessionManager::new(temp_dir, 24, 2);

        // 创建两个上传中的会话
        let mut session1 = manager
            .create_session("/test/file1.txt".to_string(), 1000)
            .await
            .unwrap();
        session1.status = UploadStatus::Uploading;
        manager.update_session(session1).await.unwrap();

        let mut session2 = manager
            .create_session("/test/file2.txt".to_string(), 2000)
            .await
            .unwrap();
        session2.status = UploadStatus::Uploading;
        manager.update_session(session2).await.unwrap();

        // 第三个会话应该失败
        let result = manager
            .create_session("/test/file3.txt".to_string(), 3000)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_session_manager_cleanup_expired() {
        let temp_dir = std::env::temp_dir().join("webdav_upload_test3");
        let manager = UploadSessionManager::new(temp_dir, 24, 10);

        // 创建一个过期的会话
        let session = UploadSession::new("/test/file.txt".to_string(), 1000, -1); // -1小时表示已过期
        let session_id = session.session_id.clone();
        let mut sessions = manager.sessions.write().await;
        sessions.insert(session_id.clone(), session);
        drop(sessions);

        // 清理过期会话
        let count = manager.cleanup_expired_sessions().await;
        assert_eq!(count, 1);

        // 验证会话已被删除
        let retrieved = manager.get_session(&session_id).await;
        assert!(retrieved.is_none());
    }
}
