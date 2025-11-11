use super::{WebDavHandler, constants::*};
use silent::prelude::*;
use std::sync::Arc;

fn register_webdav_methods(route: Route, handler: Arc<WebDavHandler>) -> Route {
    route
        .insert_handler(Method::HEAD, handler.clone())
        .insert_handler(Method::GET, handler.clone())
        .insert_handler(Method::POST, handler.clone())
        .insert_handler(Method::PUT, handler.clone())
        .insert_handler(Method::DELETE, handler.clone())
        .insert_handler(Method::OPTIONS, handler.clone())
        .insert_handler(
            Method::from_bytes(METHOD_PROPFIND).unwrap(),
            handler.clone(),
        )
        .insert_handler(
            Method::from_bytes(METHOD_PROPPATCH).unwrap(),
            handler.clone(),
        )
        .insert_handler(Method::from_bytes(METHOD_MKCOL).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_MOVE).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_COPY).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_LOCK).unwrap(), handler.clone())
        .insert_handler(
            Method::from_bytes(METHOD_VERSION_CONTROL).unwrap(),
            handler.clone(),
        )
        .insert_handler(Method::from_bytes(METHOD_REPORT).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_SEARCH).unwrap(), handler.clone())
        .insert_handler(Method::from_bytes(METHOD_UNLOCK).unwrap(), handler)
}

pub fn create_webdav_routes(
    storage: Arc<crate::storage::StorageManager>,
    notifier: Option<Arc<crate::notify::EventNotifier>>,
    sync_manager: Arc<crate::sync::crdt::SyncManager>,
    source_http_addr: String,
    version_manager: Arc<crate::version::VersionManager>,
    search_engine: Arc<crate::search::SearchEngine>,
) -> Route {
    let handler = Arc::new(WebDavHandler::new(
        storage,
        notifier,
        sync_manager,
        "".to_string(),
        source_http_addr,
        version_manager,
        search_engine,
    ));
    let root_route = register_webdav_methods(Route::new(""), handler.clone());
    let path_route = register_webdav_methods(Route::new("<path:**>"), handler);
    root_route.append(path_route)
}
