use crate::models::{EventType, FileEvent};
use crate::notify::EventNotifier;
use crate::storage::StorageManager;
use tonic::{Request, Response, Status};

// 引入生成的 protobuf 代码
pub mod file_service {
    tonic::include_proto!("silent.nas");
}

use file_service::file_service_server::{FileService, FileServiceServer};
use file_service::*;

pub struct FileServiceImpl {
    storage: StorageManager,
    notifier: Option<EventNotifier>,
    /// 对外可访问的 HTTP 基址（用于事件中携带源地址，便于其他节点拉取）
    source_http_addr: Option<String>,
}

impl FileServiceImpl {
    pub fn new(
        storage: StorageManager,
        notifier: Option<EventNotifier>,
        source_http_addr: Option<String>,
    ) -> Self {
        Self {
            storage,
            notifier,
            source_http_addr,
        }
    }

    pub fn into_server(self) -> FileServiceServer<Self> {
        FileServiceServer::new(self)
    }
}

#[tonic::async_trait]
impl FileService for FileServiceImpl {
    async fn upload_file(
        &self,
        request: Request<UploadFileRequest>,
    ) -> std::result::Result<Response<UploadFileResponse>, Status> {
        let req = request.into_inner();

        if req.file_id.is_empty() {
            return Err(Status::invalid_argument("文件 ID 不能为空"));
        }

        let metadata = self
            .storage
            .save_file(&req.file_id, &req.data)
            .await
            .map_err(|e| Status::internal(format!("保存文件失败: {}", e)))?;

        // 发布文件创建事件
        let mut event = FileEvent::new(
            EventType::Created,
            req.file_id.clone(),
            Some(metadata.clone()),
        );
        if let Some(addr) = &self.source_http_addr {
            event.source_http_addr = Some(addr.clone());
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }

        Ok(Response::new(UploadFileResponse {
            metadata: Some(convert_metadata(&metadata)),
        }))
    }

    async fn download_file(
        &self,
        request: Request<DownloadFileRequest>,
    ) -> std::result::Result<Response<DownloadFileResponse>, Status> {
        let req = request.into_inner();

        let data = self
            .storage
            .read_file(&req.file_id)
            .await
            .map_err(|e| Status::not_found(format!("文件不存在: {}", e)))?;

        let metadata = self
            .storage
            .get_metadata(&req.file_id)
            .await
            .map_err(|e| Status::internal(format!("获取元数据失败: {}", e)))?;

        Ok(Response::new(DownloadFileResponse {
            data,
            metadata: Some(convert_metadata(&metadata)),
        }))
    }

    async fn delete_file(
        &self,
        request: Request<DeleteFileRequest>,
    ) -> std::result::Result<Response<DeleteFileResponse>, Status> {
        let req = request.into_inner();

        self.storage
            .delete_file(&req.file_id)
            .await
            .map_err(|e| Status::internal(format!("删除文件失败: {}", e)))?;

        // 发布文件删除事件
        let mut event = FileEvent::new(EventType::Deleted, req.file_id.clone(), None);
        if let Some(addr) = &self.source_http_addr {
            event.source_http_addr = Some(addr.clone());
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_deleted(event).await;
        }

        Ok(Response::new(DeleteFileResponse { success: true }))
    }

    async fn get_metadata(
        &self,
        request: Request<GetMetadataRequest>,
    ) -> std::result::Result<Response<GetMetadataResponse>, Status> {
        let req = request.into_inner();

        let metadata = self
            .storage
            .get_metadata(&req.file_id)
            .await
            .map_err(|e| Status::not_found(format!("获取元数据失败: {}", e)))?;

        Ok(Response::new(GetMetadataResponse {
            metadata: Some(convert_metadata(&metadata)),
        }))
    }

    async fn list_files(
        &self,
        _request: Request<ListFilesRequest>,
    ) -> std::result::Result<Response<ListFilesResponse>, Status> {
        let files = self
            .storage
            .list_files()
            .await
            .map_err(|e| Status::internal(format!("列出文件失败: {}", e)))?;

        let files: Vec<FileMetadata> = files.iter().map(convert_metadata).collect();

        Ok(Response::new(ListFilesResponse { files }))
    }
}

/// 转换内部元数据到 protobuf 格式
fn convert_metadata(metadata: &crate::models::FileMetadata) -> FileMetadata {
    FileMetadata {
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        path: metadata.path.clone(),
        size: metadata.size,
        hash: metadata.hash.clone(),
        created_at: metadata.created_at.to_string(),
        modified_at: metadata.modified_at.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;

    #[test]
    fn test_convert_metadata() {
        let metadata = crate::models::FileMetadata {
            id: "test-id".to_string(),
            name: "test.txt".to_string(),
            path: "/path/to/test.txt".to_string(),
            size: 1024,
            hash: "abc123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let proto_metadata = convert_metadata(&metadata);

        assert_eq!(proto_metadata.id, "test-id");
        assert_eq!(proto_metadata.name, "test.txt");
        assert_eq!(proto_metadata.path, "/path/to/test.txt");
        assert_eq!(proto_metadata.size, 1024);
        assert_eq!(proto_metadata.hash, "abc123");
        assert!(!proto_metadata.created_at.is_empty());
        assert!(!proto_metadata.modified_at.is_empty());
    }

    #[test]
    fn test_convert_metadata_empty_fields() {
        let metadata = crate::models::FileMetadata {
            id: "".to_string(),
            name: "".to_string(),
            path: "".to_string(),
            size: 0,
            hash: "".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let proto_metadata = convert_metadata(&metadata);

        assert_eq!(proto_metadata.id, "");
        assert_eq!(proto_metadata.name, "");
        assert_eq!(proto_metadata.path, "");
        assert_eq!(proto_metadata.size, 0);
        assert_eq!(proto_metadata.hash, "");
    }

    #[test]
    fn test_convert_metadata_large_size() {
        let metadata = crate::models::FileMetadata {
            id: "large-file".to_string(),
            name: "large.bin".to_string(),
            path: "/data/large.bin".to_string(),
            size: 10_737_418_240, // 10GB
            hash: "hash_of_large_file".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let proto_metadata = convert_metadata(&metadata);

        assert_eq!(proto_metadata.size, 10_737_418_240);
    }

    #[test]
    fn test_convert_metadata_special_characters() {
        let metadata = crate::models::FileMetadata {
            id: "id-with-特殊字符-🔥".to_string(),
            name: "文件名.txt".to_string(),
            path: "/路径/to/文件.txt".to_string(),
            size: 2048,
            hash: "hash123".to_string(),
            created_at: Local::now().naive_local(),
            modified_at: Local::now().naive_local(),
        };

        let proto_metadata = convert_metadata(&metadata);

        assert_eq!(proto_metadata.id, "id-with-特殊字符-🔥");
        assert_eq!(proto_metadata.name, "文件名.txt");
        assert!(proto_metadata.path.contains("路径"));
    }

    #[test]
    fn test_convert_metadata_timestamp_format() {
        let now = Local::now().naive_local();
        let metadata = crate::models::FileMetadata {
            id: "test".to_string(),
            name: "test.txt".to_string(),
            path: "/test.txt".to_string(),
            size: 100,
            hash: "hash".to_string(),
            created_at: now,
            modified_at: now,
        };

        let proto_metadata = convert_metadata(&metadata);

        // 验证时间戳被转换为字符串
        assert!(!proto_metadata.created_at.is_empty());
        assert!(!proto_metadata.modified_at.is_empty());

        // 时间戳字符串应该包含日期格式
        assert!(proto_metadata.created_at.contains('-') || proto_metadata.created_at.contains(':'));
    }

    #[test]
    fn test_multiple_convert_metadata() {
        let metadatas = [
            crate::models::FileMetadata {
                id: "1".to_string(),
                name: "file1.txt".to_string(),
                path: "/file1.txt".to_string(),
                size: 100,
                hash: "hash1".to_string(),
                created_at: Local::now().naive_local(),
                modified_at: Local::now().naive_local(),
            },
            crate::models::FileMetadata {
                id: "2".to_string(),
                name: "file2.txt".to_string(),
                path: "/file2.txt".to_string(),
                size: 200,
                hash: "hash2".to_string(),
                created_at: Local::now().naive_local(),
                modified_at: Local::now().naive_local(),
            },
        ];

        let proto_metadatas: Vec<_> = metadatas.iter().map(convert_metadata).collect();

        assert_eq!(proto_metadatas.len(), 2);
        assert_eq!(proto_metadatas[0].id, "1");
        assert_eq!(proto_metadatas[1].id, "2");
    }
}
