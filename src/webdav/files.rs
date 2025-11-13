use super::{WebDavHandler, constants::*};
use crate::models::{EventType, FileEvent};
use http_body_util::BodyExt;
use silent::prelude::*;
use silent_nas_core::StorageManager as StorageManagerTrait;
use std::path::Path;
use tokio::fs;

impl WebDavHandler {
    fn insert_header_case(headers: &mut http::HeaderMap, name: &str, value: &str) {
        // 尝试以原始大小写写入（若底层实现不接受，则回退小写）
        let name_upper = http::header::HeaderName::from_bytes(name.as_bytes())
            .or_else(|_| http::header::HeaderName::from_bytes(name.to_ascii_lowercase().as_bytes()))
            .expect("invalid header name");
        if let Ok(val) = http::HeaderValue::from_str(value) {
            headers.insert(name_upper, val);
        }
    }
    pub(super) async fn handle_options(&self) -> silent::Result<Response> {
        let mut resp = Response::empty();
        // 设置 Finder 期望的大小写：DAV / Allow / Server
        Self::insert_header_case(resp.headers_mut(), "DAV", HEADER_DAV_VALUE);
        Self::insert_header_case(resp.headers_mut(), "Allow", HEADER_ALLOW_VALUE);
        Self::insert_header_case(resp.headers_mut(), "Server", "SilentWebDAV/0.1");
        // 显式 Content-Length: 0，提升部分客户端兼容性
        resp.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_static("0"),
        );
        Ok(resp)
    }

    /// 处理 WebDAV SEARCH 请求（RFC 5323）
    pub(super) async fn handle_search(&self, req: &mut Request) -> silent::Result<Response> {
        tracing::debug!("处理 WebDAV SEARCH 请求");

        // 读取请求体
        let mut body = req.take_body();
        let body_bytes = if let Some(Ok(frame)) = body.frame().await {
            frame.into_data().unwrap_or_default()
        } else {
            bytes::Bytes::new()
        };
        let body_bytes = body_bytes.to_vec();

        // 解析搜索条件
        let search_query = if !body_bytes.is_empty() {
            self.parse_search_request(&body_bytes)?
        } else {
            // 如果没有请求体，返回所有资源
            "".to_string()
        };

        tracing::debug!("搜索查询: {}", search_query);

        // 执行搜索
        let results = self
            .search_engine
            .search(&search_query, 100, 0)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("搜索失败: {}", e),
                )
            })?;

        // 构建 WebDAV multistatus 响应
        let multistatus = self.build_search_multistatus(&results)?;

        // 返回响应
        let mut response = Response::empty();
        response.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        response.set_body(full(multistatus.into_bytes()));
        Ok(response)
    }

    /// 解析 WebDAV SEARCH 请求体
    fn parse_search_request(&self, body: &[u8]) -> silent::Result<String> {
        // 简化的解析：提取 <D:searchrequest> 中的文本内容
        let body_str = String::from_utf8_lossy(body);

        // 查找 <D:searchrequest> 标签
        if let Some(start) = body_str.find("<D:searchrequest") {
            let remaining = &body_str[start..];
            if let Some(end_tag_pos) = remaining.find('>') {
                let content_start = start + end_tag_pos + 1;
                if let Some(end_pos) = body_str[content_start..].find("</D:searchrequest>") {
                    let search_content = &body_str[content_start..content_start + end_pos];
                    // 提取文本内容（简化实现）
                    let text = search_content
                        .replace("<D:select>", "")
                        .replace("</D:select>", "")
                        .replace("<D:where>", "")
                        .replace("</D:where>", "")
                        .replace("<D:and>", "")
                        .replace("</D:and>", "")
                        .replace("<D:or>", "")
                        .replace("</D:or>", "")
                        .replace("<D:not>", "")
                        .replace("</D:not>", "")
                        .replace("<D:like>", "")
                        .replace("</D:like>", "")
                        .replace("<D:prop>", "")
                        .replace("</D:prop>", "")
                        .replace("<D:literal>", "")
                        .replace("</D:literal>", "")
                        .replace("<D:caseless>", "")
                        .replace("</D:caseless>", "")
                        .replace("<D:text>", "")
                        .replace("</D:text>", "")
                        .replace("<D:collation> i;octet </D:collation>", "")
                        .replace("<D:propname>", "")
                        .replace("</D:propname>", "")
                        .replace("<D:allprop>", "")
                        .replace("</D:allprop>", "")
                        .replace("<D:getcontenttype>", "")
                        .replace("</D:getcontenttype>", "")
                        .replace("<D:getcontentlength>", "")
                        .replace("</D:getcontentlength>", "")
                        .replace("<D:displayname>", "")
                        .replace("</D:displayname>", "")
                        .replace("<D:creationdate>", "")
                        .replace("</D:creationdate>", "")
                        .replace("<D:getlastmodified>", "")
                        .replace("</D:getlastmodified>", "")
                        .replace("<D:resourcetype>", "")
                        .replace("</D:resourcetype>", "")
                        .replace("<D:collection/>", "")
                        .replace("<D:href>", "")
                        .replace("</D:href>", "")
                        .replace("\n", " ")
                        .replace("\r", " ")
                        .replace("\t", " ")
                        .trim()
                        .to_string();

                    return Ok(text);
                }
            }
        }

        // 如果解析失败，返回空字符串
        Ok("".to_string())
    }

    /// 构建搜索结果的 multistatus 响应
    fn build_search_multistatus(
        &self,
        results: &[crate::search::SearchResult],
    ) -> silent::Result<String> {
        let mut xml = String::new();
        xml.push_str(XML_HEADER);
        xml.push('\n');
        xml.push_str(XML_NS_DAV);
        xml.push('\n');

        for result in results {
            xml.push_str("  <D:response>\n");

            // href - 资源URL
            let href = format!("/api/files/{}", result.file_id);
            xml.push_str(&format!(
                "    <D:href>{}</D:href>\n",
                Self::escape_xml(&href)
            ));

            // status
            xml.push_str("    <D:status>HTTP/1.1 200 OK</D:status>\n");

            // propstat
            xml.push_str("    <D:propstat>\n");
            xml.push_str("      <D:prop>\n");

            // displayname
            if !result.name.is_empty() {
                xml.push_str(&format!(
                    "        <D:displayname>{}</D:displayname>\n",
                    Self::escape_xml(&result.name)
                ));
            }

            // getcontentlength
            if result.size > 0 {
                xml.push_str(&format!(
                    "        <D:getcontentlength>{}</D:getcontentlength>\n",
                    result.size
                ));
            }

            // getlastmodified
            if result.modified_at > 0 {
                let dt =
                    chrono::DateTime::from_timestamp(result.modified_at, 0).unwrap_or_default();
                xml.push_str(&format!(
                    "        <D:getlastmodified>{}</D:getlastmodified>\n",
                    dt.to_rfc2822()
                ));
            }

            // resourcetype
            xml.push_str("        <D:resourcetype><D:collection/></D:resourcetype>\n");

            xml.push_str("      </D:prop>\n");
            xml.push_str("      <D:status>HTTP/1.1 200 OK</D:status>\n");
            xml.push_str("    </D:propstat>\n");

            xml.push_str("  </D:response>\n");
        }

        xml.push_str(XML_MULTISTATUS_END);

        Ok(xml)
    }

    /// XML转义
    fn escape_xml(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
            .replace('\'', "&apos;")
    }

    pub(super) async fn handle_propfind(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let depth_owned = req
            .headers()
            .get("Depth")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("0")
            .to_string();

        tracing::debug!(
            "PROPFIND path='{}' depth='{}' user-agent={:?}",
            path,
            depth_owned.as_str(),
            req.headers().get("User-Agent")
        );

        // 解析请求体中的 <D:prop> 选择与 xmlns 前缀映射
        let (props_filter, ns_echo_map) = {
            let body = req.take_body();
            let xml_bytes = match body {
                ReqBody::Incoming(b) => b
                    .collect()
                    .await
                    .map_err(|e| {
                        SilentError::business_error(
                            StatusCode::BAD_REQUEST,
                            format!("读取请求体失败: {}", e),
                        )
                    })?
                    .to_bytes()
                    .to_vec(),
                ReqBody::Once(bytes) => bytes.to_vec(),
                ReqBody::Empty => Vec::new(),
            };
            WebDavHandler::parse_prop_filter_and_nsmap(&xml_bytes)
        };

        let storage_path = crate::storage::storage().get_full_path(&path);
        let metadata = fs::metadata(&storage_path).await.map_err(|e| {
            // macOS 系统文件和元数据文件不存在是正常的，只记录 debug 日志
            let is_macos_metadata = path.starts_with("/._.")
                || path.starts_with("/._")
                || path.starts_with("/.metadata_")
                || path.starts_with("/.Spotlight-")
                || path.starts_with("/.hidden")
                || path.starts_with("/.Trash");

            if is_macos_metadata {
                tracing::debug!(
                    "PROPFIND macOS 元数据文件不存在（正常）: {} -> {:?}",
                    path,
                    storage_path
                );
            } else {
                tracing::warn!(
                    "PROPFIND 路径不存在: {} -> {:?}, error: {}",
                    path,
                    storage_path,
                    e
                );
            }
            SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在")
        })?;

        tracing::debug!(
            "PROPFIND metadata: is_dir={}, len={}",
            metadata.is_dir(),
            metadata.len()
        );

        let mut xml = String::new();
        xml.push_str(XML_HEADER);
        xml.push_str(XML_NS_DAV);
        if metadata.is_dir() {
            let full_href = self.build_full_href(&path);
            self.add_prop_response_with_filter(
                &mut xml,
                &full_href,
                &storage_path,
                true,
                props_filter.as_ref(),
                Some(&ns_echo_map),
            )
            .await;
            if depth_owned.as_str() != "0" {
                if depth_owned.as_str().eq_ignore_ascii_case("infinity") {
                    self.walk_propfind_recursive(&storage_path, &path, &mut xml)
                        .await?;
                } else {
                    let mut entries = fs::read_dir(&storage_path).await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录失败: {}", e),
                        )
                    })?;
                    while let Some(entry) = entries.next_entry().await.map_err(|e| {
                        SilentError::business_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            format!("读取目录项失败: {}", e),
                        )
                    })? {
                        let entry_path = entry.path();
                        let relative_path = if path.is_empty() || path == "/" {
                            format!("/{}", entry.file_name().to_string_lossy())
                        } else {
                            format!("{}/{}", path, entry.file_name().to_string_lossy())
                        };
                        let full_href = self.build_full_href(&relative_path);
                        let is_dir = entry_path.is_dir();
                        self.add_prop_response_with_filter(
                            &mut xml,
                            &full_href,
                            &entry_path,
                            is_dir,
                            props_filter.as_ref(),
                            Some(&ns_echo_map),
                        )
                        .await;
                    }
                }
            }
        } else {
            let full_href = self.build_full_href(&path);
            self.add_prop_response_with_filter(
                &mut xml,
                &full_href,
                &storage_path,
                false,
                props_filter.as_ref(),
                Some(&ns_echo_map),
            )
            .await;
        }
        xml.push_str(XML_MULTISTATUS_END);

        // 添加调试日志，查看实际返回的 XML 内容
        tracing::debug!(
            "PROPFIND {} Depth:{} XML: {}",
            path,
            depth_owned.as_str(),
            xml
        );

        let mut resp = Response::text(&xml);
        resp.set_status(StatusCode::MULTI_STATUS);
        resp.headers_mut().insert(
            http::header::CONTENT_TYPE,
            http::HeaderValue::from_static(CONTENT_TYPE_XML),
        );
        // 额外补充 Server 头,提升 Finder 兼容性
        Self::insert_header_case(resp.headers_mut(), "Server", "SilentWebDAV/0.1");
        // 在 PROPFIND 中也返回 DAV/Allow，部分 Finder 版本会检查
        Self::insert_header_case(resp.headers_mut(), "DAV", HEADER_DAV_VALUE);
        Self::insert_header_case(resp.headers_mut(), "Allow", HEADER_ALLOW_VALUE);
        // 显式设置 Content-Length 满足严格客户端（例如 Finder）
        if let Ok(len) = http::HeaderValue::from_str(&xml.len().to_string()) {
            resp.headers_mut().insert(http::header::CONTENT_LENGTH, len);
        }
        Ok(resp)
    }

    pub(super) async fn add_prop_response(
        &self,
        xml: &mut String,
        href: &str,
        path: &Path,
        is_dir: bool,
    ) {
        self.add_prop_response_with_filter(xml, href, path, is_dir, None, None)
            .await;
    }

    pub(super) async fn add_prop_response_with_filter(
        &self,
        xml: &mut String,
        href: &str,
        path: &Path,
        is_dir: bool,
        props_filter: Option<&std::collections::HashSet<String>>,
        ns_echo: Option<&std::collections::HashMap<String, String>>, // uri -> preferred prefix
    ) {
        let metadata = match fs::metadata(path).await {
            Ok(m) => m,
            Err(_) => return,
        };
        // Finder 等客户端希望在 <D:href> 中看到未百分号编码的路径
        // 且目录以尾斜杠结尾
        let mut href_with_slash = href.to_string();
        if is_dir && !href_with_slash.ends_with('/') {
            href_with_slash.push('/');
        }
        xml.push_str("<D:response>");
        xml.push_str(&format!("<D:href>{}</D:href>", href_with_slash));
        xml.push_str("<D:propstat>");
        xml.push_str("<D:prop>");

        // displayname - 必须在最前面，macOS Finder 严格要求
        if props_filter.is_none() || props_filter.unwrap().contains("displayname") {
            let displayname = if href_with_slash == "/" {
                "/".to_string()
            } else {
                let s = href_with_slash.trim_end_matches('/');
                s.rsplit('/').next().unwrap_or(s).to_string()
            };
            xml.push_str(&format!("<D:displayname>{}</D:displayname>", displayname));
        }

        // resourcetype - 必须明确声明集合类型，macOS Finder 严格检查
        if is_dir {
            if props_filter.is_none() || props_filter.unwrap().contains("resourcetype") {
                xml.push_str("<D:resourcetype><D:collection/></D:resourcetype>");
            }
            // 兼容 Finder：目录不返回 getcontentlength
            // 为目录生成动态 ETag，基于目录内容
            if (props_filter.is_none() || props_filter.unwrap().contains("getetag"))
                && let Some(etag) = Self::calc_dir_etag(path).await
            {
                xml.push_str(&format!("<D:getetag>{}</D:getetag>", etag));
            }
        } else {
            if props_filter.is_none() || props_filter.unwrap().contains("resourcetype") {
                xml.push_str("<D:resourcetype/>");
            }
            if props_filter.is_none() || props_filter.unwrap().contains("getcontentlength") {
                xml.push_str(&format!(
                    "<D:getcontentlength>{}</D:getcontentlength>",
                    metadata.len()
                ));
            }
            if (props_filter.is_none() || props_filter.unwrap().contains("getcontenttype"))
                && let Some(ext) = path.extension()
            {
                let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                xml.push_str(&format!("<D:getcontenttype>{}</D:getcontenttype>", mime));
            }
            if (props_filter.is_none() || props_filter.unwrap().contains("getetag"))
                && let Some(etag) = Self::calc_etag_from_meta(&metadata)
            {
                xml.push_str(&format!("<D:getetag>{}</D:getetag>", etag));
            }
        }
        // creationdate（尽量取文件创建时间，否则回退到修改时间）
        let creation_dt = if let Ok(created) = metadata.created()
            && let Ok(dur) = created.duration_since(std::time::UNIX_EPOCH)
            && let Some(dt) = chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)
        {
            Some(dt)
        } else if let Ok(modified) = metadata.modified()
            && let Ok(dur) = modified.duration_since(std::time::UNIX_EPOCH)
            && let Some(dt) = chrono::DateTime::from_timestamp(dur.as_secs() as i64, 0)
        {
            Some(dt)
        } else {
            None
        };
        if (props_filter.is_none() || props_filter.unwrap().contains("creationdate"))
            && let Some(dt) = creation_dt
        {
            xml.push_str(&format!(
                "<D:creationdate>{}</D:creationdate>",
                dt.format("%Y-%m-%dT%H:%M:%SZ")
            ));
        }

        // getlastmodified - 使用文件系统的实际修改时间
        if (props_filter.is_none() || props_filter.unwrap().contains("getlastmodified"))
            && let Ok(modified) = metadata.modified()
            && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            let timestamp = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0);
            if let Some(dt) = timestamp {
                xml.push_str(&format!(
                    "<D:getlastmodified>{}</D:getlastmodified>",
                    dt.format("%a, %d %b %Y %H:%M:%S GMT")
                ));
            }
        }
        // 扩展属性（自定义属性）：仅输出结构化键 ns:{URI}#{local}
        let key_for_props = if is_dir {
            href.trim_end_matches('/').to_string()
        } else {
            href.to_string()
        };
        if let Some(map) = self.props.read().await.get(&key_for_props) {
            for (k, v) in map {
                if let Some(rest) = k.strip_prefix("ns:")
                    && let Some((uri, local)) = rest.split_once('#')
                {
                    let val = v.as_str();
                    let esc = WebDavHandler::xml_escape(val);
                    // 选择回显前缀：客户端声明的优先；避免使用 D/d
                    let mut pfx = ns_echo
                        .and_then(|m| m.get(uri).cloned())
                        .unwrap_or_else(|| "x".to_string());
                    if pfx.eq_ignore_ascii_case("d") || pfx.is_empty() {
                        pfx = "x".to_string();
                    }
                    xml.push_str(&format!(
                        "<{p}:{local} xmlns:{p}=\"{uri}\">{esc}</{p}:{local}>",
                        p = pfx,
                        local = local,
                        uri = uri,
                        esc = esc
                    ));
                }
            }
        }
        xml.push_str("</D:prop>");
        xml.push_str("<D:status>HTTP/1.1 200 OK</D:status>");
        xml.push_str("</D:propstat>");
        xml.push_str("</D:response>");
    }

    pub(super) fn xml_escape(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for ch in s.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                _ => out.push(ch),
            }
        }
        out
    }

    fn calc_etag_from_meta(metadata: &std::fs::Metadata) -> Option<String> {
        let len = metadata.len();
        let ts = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(format!("\"{}-{}\"", len, ts))
    }

    async fn calc_dir_etag(dir_path: &Path) -> Option<String> {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        let mut count = 0u64;

        // 读取目录内容并计算哈希
        let mut entries = match fs::read_dir(dir_path).await {
            Ok(e) => e,
            Err(_) => return None,
        };

        let mut names = Vec::new();

        // 即使某些 entry 读取失败，也继续处理其他的
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            names.push(name);
        }

        // 排序以确保一致性
        names.sort();

        for name in names {
            count += 1;
            name.hash(&mut hasher);
        }

        let hash = hasher.finish();
        Some(format!("\"{}-{}\"", count, hash))
    }

    pub(super) async fn walk_propfind_recursive(
        &self,
        storage_dir: &Path,
        relative_dir: &str,
        xml: &mut String,
    ) -> silent::Result<()> {
        let mut stack: Vec<(std::path::PathBuf, String)> =
            vec![(storage_dir.to_path_buf(), relative_dir.to_string())];
        while let Some((dir_path, rel_path)) = stack.pop() {
            let mut entries = fs::read_dir(&dir_path).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取目录失败: {}", e),
                )
            })?;
            while let Some(entry) = entries.next_entry().await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("读取目录项失败: {}", e),
                )
            })? {
                let entry_path = entry.path();
                let relative_path = if rel_path.is_empty() || rel_path == "/" {
                    format!("/{}", entry.file_name().to_string_lossy())
                } else {
                    format!("{}/{}", rel_path, entry.file_name().to_string_lossy())
                };
                let full_href = self.build_full_href(&relative_path);
                let is_dir = entry_path.is_dir();
                self.add_prop_response(xml, &full_href, &entry_path, is_dir)
                    .await;
                if is_dir {
                    stack.push((entry_path, relative_path));
                }
            }
        }
        Ok(())
    }

    pub(super) async fn handle_head(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = crate::storage::storage().get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;
        let mut resp = Response::empty();
        if metadata.is_dir() {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
        } else {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
            // 为提升兼容性（例如 Finder 展示大小），设置 Content-Length
            resp.headers_mut().insert(
                http::header::CONTENT_LENGTH,
                http::HeaderValue::from_str(&metadata.len().to_string()).unwrap(),
            );
            // 声明支持范围请求
            resp.headers_mut().insert(
                http::header::ACCEPT_RANGES,
                http::HeaderValue::from_static("bytes"),
            );
            if let Some(ext) = storage_path.extension() {
                let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
                resp.headers_mut().insert(
                    http::header::CONTENT_TYPE,
                    http::HeaderValue::from_str(mime.as_ref()).unwrap_or_else(|_| {
                        http::HeaderValue::from_static("application/octet-stream")
                    }),
                );
            }
            if let Some(etag) = Self::calc_etag_from_meta(&metadata) {
                if let Ok(val) = http::HeaderValue::from_str(&etag) {
                    resp.headers_mut().insert(http::header::ETAG, val);
                }
                if let Some(if_none_match) = req
                    .headers()
                    .get("If-None-Match")
                    .and_then(|h| h.to_str().ok())
                {
                    let matches = if_none_match == "*"
                        || if_none_match
                            .split(',')
                            .map(|s| s.trim())
                            .any(|t| t == etag);
                    if matches {
                        resp.set_status(StatusCode::NOT_MODIFIED);
                        return Ok(resp);
                    }
                }
            }
            if let Ok(modified) = metadata.modified()
                && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
                && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
                && let Ok(last_modified) =
                    http::HeaderValue::from_str(&dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
            {
                resp.headers_mut()
                    .insert(http::header::LAST_MODIFIED, last_modified);
            }
        }
        Ok(resp)
    }

    pub(super) async fn handle_get(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = crate::storage::storage().get_full_path(&path);
        let metadata = fs::metadata(&storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "文件不存在"))?;
        if metadata.is_dir() {
            let mut resp = Response::empty();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static(CONTENT_TYPE_HTML),
            );
            resp.set_body(full(b"<!DOCTYPE html><html><body><h1>Directory</h1><p>Use PROPFIND to list contents.</p></body></html>".to_vec()));
            return Ok(resp);
        }
        if let Some(etag) = Self::calc_etag_from_meta(&metadata)
            && let Some(if_none_match) = req
                .headers()
                .get("If-None-Match")
                .and_then(|h| h.to_str().ok())
        {
            let matches = if_none_match == "*"
                || if_none_match
                    .split(',')
                    .map(|s| s.trim())
                    .any(|t| t == etag);
            if matches {
                let mut resp = Response::empty();
                if let Ok(val) = http::HeaderValue::from_str(&etag) {
                    resp.headers_mut().insert(http::header::ETAG, val);
                }
                if let Ok(modified) = metadata.modified()
                    && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
                    && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
                    && let Ok(last_modified) = http::HeaderValue::from_str(
                        &dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string(),
                    )
                {
                    resp.headers_mut()
                        .insert(http::header::LAST_MODIFIED, last_modified);
                }
                resp.set_status(StatusCode::NOT_MODIFIED);
                return Ok(resp);
            }
        }
        let data = fs::read(&storage_path).await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("读取文件失败: {}", e),
            )
        })?;
        let mut resp = Response::empty();
        if let Some(ext) = storage_path.extension() {
            let mime = mime_guess::from_ext(&ext.to_string_lossy()).first_or_octet_stream();
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_str(mime.as_ref())
                    .unwrap_or_else(|_| http::HeaderValue::from_static("application/octet-stream")),
            );
        } else {
            resp.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/octet-stream"),
            );
        }
        resp.headers_mut().insert(
            http::header::CONTENT_LENGTH,
            http::HeaderValue::from_str(&data.len().to_string()).unwrap(),
        );
        // 声明支持范围请求，提升客户端兼容性（如 Finder）
        resp.headers_mut().insert(
            http::header::ACCEPT_RANGES,
            http::HeaderValue::from_static("bytes"),
        );
        if let Some(etag) = Self::calc_etag_from_meta(&metadata)
            && let Ok(val) = http::HeaderValue::from_str(&etag)
        {
            resp.headers_mut().insert(http::header::ETAG, val);
        }
        if let Ok(modified) = metadata.modified()
            && let Ok(datetime) = modified.duration_since(std::time::UNIX_EPOCH)
            && let Some(dt) = chrono::DateTime::from_timestamp(datetime.as_secs() as i64, 0)
            && let Ok(last_modified) =
                http::HeaderValue::from_str(&dt.format("%a, %d %b %Y %H:%M:%S GMT").to_string())
        {
            resp.headers_mut()
                .insert(http::header::LAST_MODIFIED, last_modified);
        }
        resp.set_body(full(data));
        Ok(resp)
    }

    pub(super) async fn handle_put(
        &self,
        path: &str,
        req: &mut Request,
    ) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;

        // 检查文件是否已存在，用于确定返回状态码
        let storage_path = crate::storage::storage().get_full_path(&path);
        let file_exists = storage_path.exists();

        tracing::debug!(
            "PUT path='{}' exists={} user-agent={:?}",
            path,
            file_exists,
            req.headers().get("User-Agent")
        );

        let body = req.take_body();
        let body_data = match body {
            ReqBody::Incoming(body) => body
                .collect()
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::BAD_REQUEST,
                        format!("读取请求体失败: {}", e),
                    )
                })?
                .to_bytes()
                .to_vec(),
            ReqBody::Once(bytes) => bytes.to_vec(),
            ReqBody::Empty => {
                return Err(SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    "请求体为空",
                ));
            }
        };

        if let Some(parent) = storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目录失败: {}", e),
                )
            })?;
        }

        let metadata = crate::storage::storage()
            .save_at_path(&path, &body_data)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("写入文件失败: {}", e),
                )
            })?;

        let file_id = metadata.id.clone();
        if let Err(e) = self
            .version_manager
            .create_version(
                &file_id,
                crate::models::FileVersion::from_metadata(&metadata, Some("webdav".to_string())),
            )
            .await
        {
            tracing::debug!("创建版本失败(可忽略): {}", e);
        }

        // 发布事件
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

        // 记录变更（用于 REPORT sync-collection 差异集）
        if file_exists {
            self.append_change("modified", &path);
        } else {
            self.append_change("created", &path);
        }

        let mut resp = Response::empty();
        // RFC 4918: 如果资源已存在则返回 204 No Content，新建则返回 201 Created
        resp.set_status(if file_exists {
            StatusCode::NO_CONTENT
        } else {
            StatusCode::CREATED
        });

        tracing::debug!(
            "PUT completed: path='{}' status={} size={}",
            path,
            if file_exists { 204 } else { 201 },
            body_data.len()
        );

        Ok(resp)
    }

    pub(super) async fn handle_delete(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;

        tracing::debug!(
            "DELETE path='{}' user-agent={:?}",
            path,
            // 从请求中获取 user-agent（这里无法直接访问 req，需要从调用处传入）
            "N/A"
        );

        let storage_path = crate::storage::storage().get_full_path(&path);
        let metadata = fs::metadata(&storage_path).await.map_err(|e| {
            tracing::warn!(
                "DELETE 文件不存在: {} -> {:?}, error: {}",
                path,
                storage_path,
                e
            );
            SilentError::business_error(StatusCode::NOT_FOUND, "路径不存在")
        })?;

        if metadata.is_dir() {
            fs::remove_dir_all(&storage_path).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除目录失败: {}", e),
                )
            })?;
        } else {
            fs::remove_file(&storage_path).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("删除文件失败: {}", e),
                )
            })?;
        }

        tracing::debug!("DELETE completed: path='{}'", path);

        let file_id = scru128::new_string();
        let mut event = FileEvent::new(EventType::Deleted, file_id, None);
        if let Ok(host) = std::env::var("ADVERTISE_HOST").or_else(|_| std::env::var("HOSTNAME")) {
            event.source_http_addr = Some(format!(
                "http://{}:{}",
                host,
                std::env::var("HTTP_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(8080)
            ));
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_deleted(event).await;
        }
        // 记录删除
        self.append_change("deleted", &path);
        let mut resp = Response::empty();
        resp.set_status(StatusCode::NO_CONTENT);
        Ok(resp)
    }

    pub(super) async fn handle_mkcol(&self, path: &str) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        let storage_path = crate::storage::storage().get_full_path(&path);
        if storage_path.exists() {
            return Err(SilentError::business_error(
                StatusCode::METHOD_NOT_ALLOWED,
                "路径已存在",
            ));
        }
        fs::create_dir_all(&storage_path).await.map_err(|e| {
            SilentError::business_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("创建目录失败: {}", e),
            )
        })?;
        // 记录创建
        self.append_change("created", &path);
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) async fn handle_move(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let dest = req
            .headers()
            .get("Destination")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少 Destination 头")
            })?;
        let dest_path = self.extract_path_from_url(dest)?;
        let storage_path = crate::storage::storage().get_full_path(&path);
        let dest_storage_path = crate::storage::storage().get_full_path(&dest_path);
        if let Some(parent) = dest_storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目标目录失败: {}", e),
                )
            })?;
        }
        fs::rename(&storage_path, &dest_storage_path)
            .await
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("移动失败: {}", e),
                )
            })?;
        // 记录为移动 from->to，供 REPORT 增量同步输出
        self.append_move(&path, &dest_path);
        // 发布事件
        let file_id = scru128::new_string();
        let mut event = FileEvent::new(EventType::Modified, file_id, None);
        if let Ok(host) = std::env::var("ADVERTISE_HOST").or_else(|_| std::env::var("HOSTNAME")) {
            event.source_http_addr = Some(format!(
                "http://{}:{}",
                host,
                std::env::var("HTTP_PORT")
                    .ok()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(8080)
            ));
        }
        if let Some(ref n) = self.notifier {
            let _ = n.notify_created(event).await;
        }
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) async fn handle_copy(&self, path: &str, req: &Request) -> silent::Result<Response> {
        let path = Self::decode_path(path)?;
        self.ensure_lock_ok(&path, req).await?;
        let dest = req
            .headers()
            .get("Destination")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                SilentError::business_error(StatusCode::BAD_REQUEST, "缺少 Destination 头")
            })?;
        let dest_path = self.extract_path_from_url(dest)?;
        let src_storage_path = crate::storage::storage().get_full_path(&path);
        let dest_storage_path = crate::storage::storage().get_full_path(&dest_path);
        let metadata = fs::metadata(&src_storage_path)
            .await
            .map_err(|_| SilentError::business_error(StatusCode::NOT_FOUND, "源路径不存在"))?;
        if let Some(parent) = dest_storage_path.parent() {
            fs::create_dir_all(parent).await.map_err(|e| {
                SilentError::business_error(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("创建目标目录失败: {}", e),
                )
            })?;
        }
        if metadata.is_dir() {
            Self::copy_dir_all(&src_storage_path, &dest_storage_path)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("复制目录失败: {}", e),
                    )
                })?;
        } else {
            fs::copy(&src_storage_path, &dest_storage_path)
                .await
                .map_err(|e| {
                    SilentError::business_error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        format!("复制文件失败: {}", e),
                    )
                })?;
        }
        // 记录创建
        self.append_change("created", &dest_path);
        let mut resp = Response::empty();
        resp.set_status(StatusCode::CREATED);
        Ok(resp)
    }

    pub(super) fn extract_path_from_url(&self, url: &str) -> silent::Result<String> {
        let path = if let Some(idx) = url.find("://") {
            if let Some(path_start) = url[idx + 3..].find('/') {
                &url[idx + 3 + path_start..]
            } else {
                "/"
            }
        } else if url.starts_with('/') {
            url
        } else {
            return Err(SilentError::business_error(
                StatusCode::BAD_REQUEST,
                "无效的目标 URL",
            ));
        };
        let relative_path = path.strip_prefix(&self.base_path).unwrap_or(path);
        urlencoding::decode(relative_path)
            .map(|s| s.to_string())
            .map_err(|e| {
                SilentError::business_error(
                    StatusCode::BAD_REQUEST,
                    format!("目标路径解码失败: {}", e),
                )
            })
    }

    pub(super) async fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
        fs::create_dir_all(dst).await?;
        let mut entries = fs::read_dir(src).await?;
        while let Some(entry) = entries.next_entry().await? {
            let ty = entry.file_type().await?;
            let src_path = entry.path();
            let dst_path = dst.join(entry.file_name());
            if ty.is_dir() {
                Box::pin(Self::copy_dir_all(&src_path, &dst_path)).await?;
            } else {
                fs::copy(&src_path, &dst_path).await?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    async fn build_handler() -> WebDavHandler {
        let dir = tempfile::tempdir().unwrap();
        let storage =
            crate::storage::StorageManager::new(dir.path().to_path_buf(), 4 * 1024 * 1024);
        let _ = crate::storage::init_global_storage(storage.clone());
        storage.init().await.unwrap();
        let syncm = crate::sync::crdt::SyncManager::new("node-test".to_string(), None);
        let ver = crate::version::VersionManager::new(
            std::sync::Arc::new(storage.clone()),
            Default::default(),
            dir.path().to_str().unwrap(),
        );
        let search_engine = Arc::new(
            crate::search::SearchEngine::new(
                dir.path().join("search_index"),
                dir.path().to_path_buf(),
            )
            .unwrap(),
        );
        WebDavHandler::new(
            None,
            syncm,
            "".into(),
            "http://127.0.0.1:8080".into(),
            ver,
            search_engine,
        )
    }

    #[tokio::test]
    async fn test_calc_etag_from_meta_and_dir_etag() {
        // 临时目录与文件
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("a.txt");
        tokio::fs::write(&file_path, b"hello").await.unwrap();

        // 文件 etag
        let meta = std::fs::metadata(&file_path).unwrap();
        let etag = WebDavHandler::calc_etag_from_meta(&meta).unwrap();
        assert!(etag.starts_with('\"') && etag.ends_with('\"'));

        // 目录 etag（非空目录）
        let detag = WebDavHandler::calc_dir_etag(dir.path()).await.unwrap();
        assert!(detag.starts_with('\"') && detag.ends_with('\"'));
    }

    #[tokio::test]
    async fn test_propfind_depth_infinity_and_head_get() {
        let handler = build_handler().await;

        // 准备目录与文件
        let root = crate::storage::storage().root_dir().to_path_buf();
        let data_root = root.join("data");
        tokio::fs::create_dir_all(data_root.join("dir/sub"))
            .await
            .unwrap();
        let fpath = crate::storage::storage().get_full_path("/dir/sub/a.txt");
        tokio::fs::write(&fpath, b"hello").await.unwrap();

        // PROPFIND Depth: infinity
        let mut req = Request::empty();
        req.headers_mut()
            .insert("Depth", http::HeaderValue::from_static("infinity"));
        let resp = handler.handle_propfind("/dir", &mut req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::MULTI_STATUS);
        // 仅校验头部存在，XML 体内容不做解析（已覆盖生成路径）
        assert_eq!(
            resp.headers()
                .get(http::header::CONTENT_TYPE)
                .unwrap()
                .to_str()
                .unwrap(),
            CONTENT_TYPE_XML
        );

        // HEAD 文件
        let head = handler
            .handle_head("/dir/sub/a.txt", &Request::empty())
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert!(head.headers().get(http::header::CONTENT_LENGTH).is_some());

        // GET If-None-Match 命中返回 304
        let meta = std::fs::metadata(&fpath).unwrap();
        let etag = WebDavHandler::calc_etag_from_meta(&meta).unwrap();
        let mut get_req = Request::empty();
        get_req
            .headers_mut()
            .insert("If-None-Match", http::HeaderValue::from_str(&etag).unwrap());
        let not_mod = handler
            .handle_get("/dir/sub/a.txt", &get_req)
            .await
            .unwrap();
        assert_eq!(not_mod.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn test_mkcol_move_copy() {
        let handler = build_handler().await;

        // MKCOL 创建目录
        let mk = handler.handle_mkcol("/mk/a").await.unwrap();
        assert_eq!(mk.status(), StatusCode::CREATED);
        assert!(crate::storage::storage().get_full_path("/mk/a").exists());

        // 创建源文件
        tokio::fs::write(
            crate::storage::storage().get_full_path("/mk/a/x.txt"),
            b"data",
        )
        .await
        .unwrap();

        // MOVE 到新路径
        let http_req = http::Request::builder()
            .method("MOVE")
            .uri("/mk/a/x.txt")
            .header("Destination", "/mk/b/y.txt")
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        let req = Request::from_parts(parts, ReqBody::Empty);
        let mv = handler.handle_move("/mk/a/x.txt", &req).await.unwrap();
        assert_eq!(mv.status(), StatusCode::CREATED);
        assert!(
            crate::storage::storage()
                .get_full_path("/mk/b/y.txt")
                .exists()
        );
        assert!(
            !crate::storage::storage()
                .get_full_path("/mk/a/x.txt")
                .exists()
        );

        // COPY 复制文件
        let http_req2 = http::Request::builder()
            .method("COPY")
            .uri("/mk/b/y.txt")
            .header("Destination", "/mk/c/z.txt")
            .body(())
            .unwrap();
        let (parts2, _) = http_req2.into_parts();
        let req2 = Request::from_parts(parts2, ReqBody::Empty);
        let cp = handler.handle_copy("/mk/b/y.txt", &req2).await.unwrap();
        assert_eq!(cp.status(), StatusCode::CREATED);
        assert!(
            crate::storage::storage()
                .get_full_path("/mk/c/z.txt")
                .exists()
        );
        assert!(
            crate::storage::storage()
                .get_full_path("/mk/b/y.txt")
                .exists()
        );
    }

    #[tokio::test]
    async fn test_propfind_depth0_and1_and_errors() {
        let handler = build_handler().await;

        // 创建文件与目录
        tokio::fs::create_dir_all(crate::storage::storage().get_full_path("/p0"))
            .await
            .unwrap();
        let f = crate::storage::storage().get_full_path("/p0/a.txt");
        tokio::fs::write(&f, b"x").await.unwrap();

        // Depth: 0 针对文件
        let mut d0 = Request::empty();
        d0.headers_mut()
            .insert("Depth", http::HeaderValue::from_static("0"));
        let r0 = handler.handle_propfind("/p0/a.txt", &mut d0).await.unwrap();
        assert_eq!(r0.status(), StatusCode::MULTI_STATUS);

        // Depth: 1 针对目录
        let mut d1 = Request::empty();
        d1.headers_mut()
            .insert("Depth", http::HeaderValue::from_static("1"));
        let r1 = handler.handle_propfind("/p0", &mut d1).await.unwrap();
        assert_eq!(r1.status(), StatusCode::MULTI_STATUS);

        // PUT 空请求体 -> 400
        let http_req = http::Request::builder()
            .method("PUT")
            .uri("/err.txt")
            .body(())
            .unwrap();
        let (parts, _) = http_req.into_parts();
        let mut req = Request::from_parts(parts, ReqBody::Empty);
        let err = handler
            .handle_put("/err.txt", &mut req)
            .await
            .err()
            .unwrap();
        assert_eq!(err.status(), StatusCode::BAD_REQUEST);

        // DELETE 不存在 -> 404
        let err2 = handler.handle_delete("/not-exist").await.err().unwrap();
        assert_eq!(err2.status(), StatusCode::NOT_FOUND);

        // MOVE/COPY 缺少 Destination -> 400
        let http_req2 = http::Request::builder()
            .method("MOVE")
            .uri("/p0/a.txt")
            .body(())
            .unwrap();
        let (p2, _) = http_req2.into_parts();
        let req2 = Request::from_parts(p2, ReqBody::Empty);
        let emv = handler.handle_move("/p0/a.txt", &req2).await.err().unwrap();
        assert_eq!(emv.status(), StatusCode::BAD_REQUEST);

        let http_req3 = http::Request::builder()
            .method("COPY")
            .uri("/p0/a.txt")
            .body(())
            .unwrap();
        let (p3, _) = http_req3.into_parts();
        let req3 = Request::from_parts(p3, ReqBody::Empty);
        let ecp = handler.handle_copy("/p0/a.txt", &req3).await.err().unwrap();
        assert_eq!(ecp.status(), StatusCode::BAD_REQUEST);

        // extract_path_from_url 无效 URL -> 400
        let e = handler.extract_path_from_url("invalid").err().unwrap();
        assert_eq!(e.status(), StatusCode::BAD_REQUEST);

        // HEAD If-None-Match 304
        let meta = std::fs::metadata(&f).unwrap();
        let etag = WebDavHandler::calc_etag_from_meta(&meta).unwrap();
        let mut hreq = Request::empty();
        hreq.headers_mut()
            .insert("If-None-Match", http::HeaderValue::from_str(&etag).unwrap());
        let head_resp = handler.handle_head("/p0/a.txt", &hreq).await.unwrap();
        assert_eq!(head_resp.status(), StatusCode::NOT_MODIFIED);
    }
}
