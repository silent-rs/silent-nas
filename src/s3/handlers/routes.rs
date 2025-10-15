use crate::notify::EventNotifier;
use crate::s3::auth::S3Auth;
use crate::s3::service::S3Service;
use crate::storage::StorageManager;
use http::Method;
use http::StatusCode;
use silent::prelude::*;
use std::sync::Arc;
use tracing::debug;

/// 创建S3路由
pub fn create_s3_routes(
    storage: Arc<StorageManager>,
    notifier: Arc<EventNotifier>,
    auth: Option<S3Auth>,
) -> Route {
    let service = Arc::new(S3Service::new(storage, notifier, auth));

    // Bucket操作 - 合并GET和HEAD
    let service_bucket = service.clone();
    let bucket_handler = move |req: Request| {
        let service = service_bucket.clone();
        async move {
            debug!("bucket_handler: method={}, uri={}", req.method(), req.uri());
            match *req.method() {
                Method::GET => {
                    // 检查查询参数决定调用哪个API
                    let query = req.uri().query().unwrap_or("");
                    if query.contains("list-type=2") {
                        service.list_objects_v2(req).await
                    } else if query.contains("location") {
                        service.get_bucket_location(req).await
                    } else if query.contains("versioning") {
                        service.get_bucket_versioning(req).await
                    } else {
                        service.list_objects(req).await
                    }
                }
                Method::HEAD => {
                    debug!("调用head_bucket");
                    service.head_bucket(req).await
                }
                _ => service.error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                ),
            }
        }
    };

    let service_put_bucket = service.clone();
    let put_bucket = move |req: Request| {
        let service = service_put_bucket.clone();
        async move { service.put_bucket(req).await }
    };

    let service_delete_bucket = service.clone();
    let delete_bucket = move |req: Request| {
        let service = service_delete_bucket.clone();
        async move { service.delete_bucket(req).await }
    };

    // 对象操作 - PUT需要区分PutObject、CopyObject和UploadPart
    let service_put = service.clone();
    let put_object = move |req: Request| {
        let service = service_put.clone();
        async move {
            let query = req.uri().query().unwrap_or("");

            // 检查是否是UploadPart请求
            if query.contains("partNumber") && query.contains("uploadId") {
                return service.upload_part(req).await;
            }

            // 检查是否是CopyObject请求（有x-amz-copy-source头）
            if req.headers().contains_key("x-amz-copy-source") {
                service.copy_object(req).await
            } else {
                service.put_object(req).await
            }
        }
    };

    let service_get_head = service.clone();
    let service_bucket_get = service.clone();
    let get_or_head_object = move |req: Request| {
        let service = service_get_head.clone();
        let service_bucket = service_bucket_get.clone();
        async move {
            // 检查key是否为空，如果为空说明是bucket级别请求
            let key_result: silent::Result<String> = req.get_path_params("key");
            if let Ok(key) = &key_result {
                if key.is_empty() {
                    // 空key，这是bucket级别请求，转发到bucket_handler逻辑
                    debug!("Empty key detected, routing to bucket handler");
                    match *req.method() {
                        Method::GET => {
                            let query = req.uri().query().unwrap_or("");
                            if query.contains("list-type=2") {
                                service_bucket.list_objects_v2(req).await
                            } else if query.contains("location") {
                                service_bucket.get_bucket_location(req).await
                            } else if query.contains("versioning") {
                                service_bucket.get_bucket_versioning(req).await
                            } else {
                                service_bucket.list_objects(req).await
                            }
                        }
                        Method::HEAD => service_bucket.head_bucket(req).await,
                        _ => service.error_response(
                            StatusCode::METHOD_NOT_ALLOWED,
                            "MethodNotAllowed",
                            "Method not allowed",
                        ),
                    }
                } else {
                    // 正常的对象请求
                    match *req.method() {
                        Method::GET => service.get_object(req).await,
                        Method::HEAD => service.head_object(req).await,
                        _ => service.error_response(
                            StatusCode::METHOD_NOT_ALLOWED,
                            "MethodNotAllowed",
                            "Method not allowed",
                        ),
                    }
                }
            } else {
                service.error_response(
                    StatusCode::BAD_REQUEST,
                    "InvalidRequest",
                    "Missing parameters",
                )
            }
        }
    };

    let service_delete = service.clone();
    let delete_object = move |req: Request| {
        let service = service_delete.clone();
        async move {
            let query = req.uri().query().unwrap_or("");

            // 检查是否是AbortMultipartUpload
            if query.contains("uploadId") {
                service.abort_multipart_upload(req).await
            } else {
                service.delete_object(req).await
            }
        }
    };

    // 根路径处理ListBuckets
    let service_root = service.clone();
    let root_handler = move |req: Request| {
        let service = service_root.clone();
        async move {
            match *req.method() {
                Method::GET => service.list_buckets(req).await,
                _ => service.error_response(
                    StatusCode::METHOD_NOT_ALLOWED,
                    "MethodNotAllowed",
                    "Method not allowed",
                ),
            }
        }
    };

    // POST处理（包括bucket和对象级别）
    let service_post = service.clone();
    let post_handler = move |req: Request| {
        let service = service_post.clone();
        async move {
            let query = req.uri().query().unwrap_or("");

            // 检查key是否为空
            let key_result: silent::Result<String> = req.get_path_params("key");
            if let Ok(key) = &key_result {
                if key.is_empty() {
                    // Bucket级别POST - DeleteObjects
                    if query.contains("delete") {
                        service.delete_objects(req).await
                    } else {
                        service.error_response(
                            StatusCode::BAD_REQUEST,
                            "InvalidRequest",
                            "Invalid POST request",
                        )
                    }
                } else {
                    // 对象级别POST
                    if query.contains("uploads") {
                        // InitiateMultipartUpload
                        service.initiate_multipart_upload(req).await
                    } else if query.contains("uploadId") {
                        // CompleteMultipartUpload
                        service.complete_multipart_upload(req).await
                    } else {
                        service.error_response(
                            StatusCode::METHOD_NOT_ALLOWED,
                            "MethodNotAllowed",
                            "Invalid POST request",
                        )
                    }
                }
            } else {
                service.error_response(
                    StatusCode::BAD_REQUEST,
                    "InvalidRequest",
                    "Missing parameters",
                )
            }
        }
    };

    Route::new_root().get(root_handler).append(
        Route::new("<bucket>")
            // Bucket级别操作 - GET、HEAD、PUT、DELETE
            .get(bucket_handler)
            .put(put_bucket)
            .delete(delete_bucket)
            // 对象级别操作（也处理空key的bucket请求）
            .append(
                Route::new("<key:**>")
                    .put(put_object)
                    .get(get_or_head_object)
                    .delete(delete_object)
                    .post(post_handler),
            ),
    )
}
