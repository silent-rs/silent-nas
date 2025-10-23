// WebDAV constants split from webdav.rs

pub const METHOD_PROPFIND: &[u8] = b"PROPFIND";
pub const METHOD_PROPPATCH: &[u8] = b"PROPPATCH";
pub const METHOD_LOCK: &[u8] = b"LOCK";
pub const METHOD_UNLOCK: &[u8] = b"UNLOCK";
pub const METHOD_MKCOL: &[u8] = b"MKCOL";
pub const METHOD_MOVE: &[u8] = b"MOVE";
pub const METHOD_COPY: &[u8] = b"COPY";
#[allow(dead_code)]
pub const METHOD_VERSION_CONTROL: &[u8] = b"VERSION-CONTROL";
#[allow(dead_code)]
pub const METHOD_REPORT: &[u8] = b"REPORT";

pub const XML_HEADER: &str = r#"<?xml version=\"1.0\" encoding=\"utf-8\"?>"#;
pub const XML_NS_DAV: &str = r#"<D:multistatus xmlns:D=\"DAV:\">"#;
pub const XML_MULTISTATUS_END: &str = "</D:multistatus>";

pub const HEADER_DAV: &str = "dav";
// 按需返回 DAV 能力集合
// 需求：OPTIONS DAV: 返回 1,2,ordered-collections
pub const HEADER_DAV_VALUE: &str = "1, 2, ordered-collections";
pub const HEADER_ALLOW_VALUE: &str = "OPTIONS, GET, HEAD, PUT, DELETE, PROPFIND, PROPPATCH, MKCOL, MOVE, COPY, LOCK, UNLOCK, VERSION-CONTROL, REPORT";
// 为兼容 Finder，使用 text/xml
pub const CONTENT_TYPE_XML: &str = "text/xml; charset=utf-8";
pub const CONTENT_TYPE_HTML: &str = "text/html; charset=utf-8";
