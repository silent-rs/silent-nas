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
    notifier: EventNotifier,
}

impl FileServiceImpl {
    pub fn new(storage: StorageManager, notifier: EventNotifier) -> Self {
        Self { storage, notifier }
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
        let event = FileEvent::new(
            EventType::Created,
            req.file_id.clone(),
            Some(metadata.clone()),
        );
        let _ = self.notifier.notify_created(event).await;

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
        let event = FileEvent::new(EventType::Deleted, req.file_id.clone(), None);
        let _ = self.notifier.notify_deleted(event).await;

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
