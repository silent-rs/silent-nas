//! 文件内容提取器
//!
//! 支持从不同文件格式中提取文本内容，包括：
//! - TXT文本文件
//! - HTML文件
//! - Markdown文件
//! - PDF文件（基础支持）
//! - 代码文件
//! - 日志文件

use crate::error::{NasError, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// 文件类型
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// 文本文件
    Text,
    /// HTML文件
    Html,
    /// Markdown文件
    Markdown,
    /// PDF文件
    Pdf,
    /// 代码文件
    Code,
    /// 日志文件
    Log,
    /// 二进制文件（不支持文本提取）
    Binary,
    /// 未知类型
    Unknown,
}

/// 内容提取结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentExtractionResult {
    /// 提取的文本内容
    pub content: String,
    /// 文件类型
    pub file_type: FileType,
    /// 内容长度
    pub content_length: usize,
    /// 编码格式
    pub encoding: String,
}

/// 内容提取器
pub struct ContentExtractor {
    /// 支持的文件扩展名映射
    extension_map: std::collections::HashMap<String, FileType>,
}

impl ContentExtractor {
    /// 创建新的内容提取器
    pub fn new() -> Self {
        let mut extension_map = std::collections::HashMap::new();

        // 文本文件
        extension_map.insert("txt".to_string(), FileType::Text);
        extension_map.insert("text".to_string(), FileType::Text);

        // HTML文件
        extension_map.insert("html".to_string(), FileType::Html);
        extension_map.insert("htm".to_string(), FileType::Html);
        extension_map.insert("xhtml".to_string(), FileType::Html);

        // Markdown文件
        extension_map.insert("md".to_string(), FileType::Markdown);
        extension_map.insert("markdown".to_string(), FileType::Markdown);

        // PDF文件（目前不完整支持）
        extension_map.insert("pdf".to_string(), FileType::Pdf);

        // 代码文件
        extension_map.insert("rs".to_string(), FileType::Code);
        extension_map.insert("rust".to_string(), FileType::Code);
        extension_map.insert("js".to_string(), FileType::Code);
        extension_map.insert("javascript".to_string(), FileType::Code);
        extension_map.insert("ts".to_string(), FileType::Code);
        extension_map.insert("typescript".to_string(), FileType::Code);
        extension_map.insert("py".to_string(), FileType::Code);
        extension_map.insert("python".to_string(), FileType::Code);
        extension_map.insert("java".to_string(), FileType::Code);
        extension_map.insert("c".to_string(), FileType::Code);
        extension_map.insert("cpp".to_string(), FileType::Code);
        extension_map.insert("cc".to_string(), FileType::Code);
        extension_map.insert("cxx".to_string(), FileType::Code);
        extension_map.insert("go".to_string(), FileType::Code);
        extension_map.insert("php".to_string(), FileType::Code);
        extension_map.insert("rb".to_string(), FileType::Code);
        extension_map.insert("sh".to_string(), FileType::Code);
        extension_map.insert("bash".to_string(), FileType::Code);
        extension_map.insert("zsh".to_string(), FileType::Code);
        extension_map.insert("json".to_string(), FileType::Code);
        extension_map.insert("yaml".to_string(), FileType::Code);
        extension_map.insert("yml".to_string(), FileType::Code);
        extension_map.insert("xml".to_string(), FileType::Code);
        extension_map.insert("toml".to_string(), FileType::Code);
        extension_map.insert("sql".to_string(), FileType::Code);

        // 日志文件
        extension_map.insert("log".to_string(), FileType::Log);
        extension_map.insert("logs".to_string(), FileType::Log);

        Self { extension_map }
    }

    /// 从文件中提取内容
    pub fn extract_content(&self, file_path: &Path) -> Result<ContentExtractionResult> {
        let file_type = self.detect_file_type(file_path)?;

        // 根据文件类型提取内容
        match file_type {
            FileType::Text | FileType::Code | FileType::Log => {
                self.extract_text_content(file_path, file_type)
            }
            FileType::Html => self.extract_html_content(file_path, file_type),
            FileType::Markdown => self.extract_markdown_content(file_path, file_type),
            FileType::Pdf => {
                // 目前PDF支持有限，仅返回提示信息
                self.extract_pdf_content(file_path, file_type)
            }
            FileType::Binary | FileType::Unknown => {
                // 不支持的内容类型，统一返回Binary
                Ok(ContentExtractionResult {
                    content: "".to_string(),
                    file_type: FileType::Binary,
                    content_length: 0,
                    encoding: "unknown".to_string(),
                })
            }
        }
    }

    /// 检测文件类型
    fn detect_file_type(&self, file_path: &Path) -> Result<FileType> {
        let extension = file_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();

        Ok(self
            .extension_map
            .get(&extension)
            .cloned()
            .unwrap_or(FileType::Unknown))
    }

    /// 提取文本内容
    fn extract_text_content(
        &self,
        file_path: &Path,
        file_type: FileType,
    ) -> Result<ContentExtractionResult> {
        // 读取文件
        let content = fs::read_to_string(file_path).map_err(|e| {
            NasError::Storage(format!("读取文件失败 {}: {}", file_path.display(), e))
        })?;

        let processed_content = self.preprocess_text(&content);

        Ok(ContentExtractionResult {
            content: processed_content.clone(),
            file_type,
            content_length: processed_content.len(),
            encoding: "utf-8".to_string(),
        })
    }

    /// 提取HTML内容
    fn extract_html_content(
        &self,
        file_path: &Path,
        file_type: FileType,
    ) -> Result<ContentExtractionResult> {
        // 读取HTML文件
        let content = fs::read_to_string(file_path)
            .map_err(|e| NasError::Storage(format!("读取HTML文件失败: {}", e)))?;

        // 简单的HTML标签移除
        let text_content = self.strip_html_tags(&content);
        let processed_content = self.preprocess_text(&text_content);

        Ok(ContentExtractionResult {
            content: processed_content.clone(),
            file_type,
            content_length: processed_content.len(),
            encoding: "utf-8".to_string(),
        })
    }

    /// 提取Markdown内容
    fn extract_markdown_content(
        &self,
        file_path: &Path,
        file_type: FileType,
    ) -> Result<ContentExtractionResult> {
        // 读取Markdown文件
        let content = fs::read_to_string(file_path)
            .map_err(|e| NasError::Storage(format!("读取Markdown文件失败: {}", e)))?;

        // 简单的Markdown解析（移除格式标记）
        let processed_content = self.preprocess_text(&content);
        // 可以进一步处理Markdown特定格式，如移除**粗体**、*斜体*等

        Ok(ContentExtractionResult {
            content: processed_content.clone(),
            file_type,
            content_length: processed_content.len(),
            encoding: "utf-8".to_string(),
        })
    }

    /// 提取PDF内容
    fn extract_pdf_content(
        &self,
        _file_path: &Path,
        file_type: FileType,
    ) -> Result<ContentExtractionResult> {
        // TODO: 集成PDF解析库（如poppler或pdf-extract）
        // 目前仅返回提示信息
        Ok(ContentExtractionResult {
            content: "PDF文件内容提取功能尚未实现".to_string(),
            file_type,
            content_length: 0,
            encoding: "unknown".to_string(),
        })
    }

    /// 移除HTML标签
    fn strip_html_tags(&self, html: &str) -> String {
        let mut result = String::new();
        let chars: Vec<char> = html.chars().collect();
        let mut i = 0;

        while i < chars.len() {
            if chars[i] == '<' {
                // 检查是否是script或style标签
                // <script 需要 7 个字符，<style 需要 6 个字符
                if i + 7 < chars.len() {
                    let tag_start = chars[i..i + 7].iter().collect::<String>().to_lowercase();
                    if tag_start.starts_with("<script") {
                        // 跳过整个script或style标签块
                        while i < chars.len() && chars[i] != '>' {
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1; // 跳过 '>'
                        }

                        // 查找对应的结束标签 </script>
                        while i < chars.len() {
                            if i + 9 <= chars.len() {
                                let end_str: String = chars[i..i + 9].iter().collect();
                                if end_str.to_lowercase() == "</script>" {
                                    i += 9;
                                    break;
                                }
                            }
                            i += 1;
                        }
                        continue;
                    }
                }
                if i + 6 < chars.len() {
                    let tag_start = chars[i..i + 6].iter().collect::<String>().to_lowercase();
                    if tag_start.starts_with("<style") {
                        // 跳过整个style标签块
                        while i < chars.len() && chars[i] != '>' {
                            i += 1;
                        }
                        if i < chars.len() {
                            i += 1; // 跳过 '>'
                        }

                        // 查找对应的结束标签 </style>
                        while i < chars.len() {
                            if i + 8 <= chars.len() {
                                let end_str: String = chars[i..i + 8].iter().collect();
                                if end_str.to_lowercase() == "</style>" {
                                    i += 8;
                                    break;
                                }
                            }
                            i += 1;
                        }
                        continue;
                    }
                }

                // 普通标签，跳过直到 '>'
                while i < chars.len() && chars[i] != '>' {
                    i += 1;
                }
                if i < chars.len() {
                    i += 1;
                }
                continue;
            }

            // 添加普通字符
            result.push(chars[i]);
            i += 1;
        }

        result
    }

    /// 文本预处理
    fn preprocess_text(&self, text: &str) -> String {
        // 移除多余的空白字符
        let text = text.replace("\r\n", "\n").replace("\r", "\n");

        // 压缩连续空格和换行为单个空格
        let text = text.chars().fold(String::new(), |mut acc, c| {
            if c.is_whitespace() {
                if !acc.chars().last().is_some_and(|last| last.is_whitespace()) {
                    acc.push(' ');
                }
            } else {
                acc.push(c);
            }
            acc
        });

        text.trim().to_string()
    }

    /// 检查文件是否支持内容提取
    #[allow(dead_code)]
    pub fn is_supported(&self, file_path: &Path) -> bool {
        let file_type = self
            .detect_file_type(file_path)
            .unwrap_or(FileType::Unknown);
        !matches!(file_type, FileType::Binary | FileType::Unknown)
    }
}

impl Default for ContentExtractor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_detect_file_type() {
        let extractor = ContentExtractor::new();

        assert_eq!(
            extractor.detect_file_type(Path::new("test.txt")).unwrap(),
            FileType::Text
        );
        assert_eq!(
            extractor.detect_file_type(Path::new("test.html")).unwrap(),
            FileType::Html
        );
        assert_eq!(
            extractor.detect_file_type(Path::new("test.md")).unwrap(),
            FileType::Markdown
        );
        assert_eq!(
            extractor.detect_file_type(Path::new("test.rs")).unwrap(),
            FileType::Code
        );
        assert_eq!(
            extractor.detect_file_type(Path::new("test.pdf")).unwrap(),
            FileType::Pdf
        );
        assert_eq!(
            extractor
                .detect_file_type(Path::new("test.unknown"))
                .unwrap(),
            FileType::Unknown
        );
    }

    #[test]
    fn test_is_supported() {
        let extractor = ContentExtractor::new();

        assert!(extractor.is_supported(Path::new("test.txt")));
        assert!(extractor.is_supported(Path::new("test.html")));
        assert!(extractor.is_supported(Path::new("test.rs")));
        assert!(!extractor.is_supported(Path::new("test.zip")));
        assert!(!extractor.is_supported(Path::new("test.jpg")));
    }

    #[test]
    fn test_extract_text_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        fs::write(&file_path, "Hello World\nThis is a test file.").unwrap();

        let extractor = ContentExtractor::new();
        let result = extractor.extract_content(&file_path).unwrap();

        assert!(result.content.contains("Hello World"));
        assert_eq!(result.file_type, FileType::Text);
        assert!(result.content_length > 0);
    }

    #[test]
    fn test_extract_html_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.html");

        let html = "<html><body><h1>Title</h1><p>Paragraph</p></body></html>";
        fs::write(&file_path, html).unwrap();

        let extractor = ContentExtractor::new();
        let result = extractor.extract_content(&file_path).unwrap();

        assert!(result.content.contains("Title"));
        assert!(result.content.contains("Paragraph"));
        assert!(!result.content.contains("<h1>"));
        assert_eq!(result.file_type, FileType::Html);
    }

    #[test]
    fn test_extract_code_content() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.rs");

        let code = r#"
fn main() {
    println!("Hello, World!");
}
"#;
        fs::write(&file_path, code).unwrap();

        let extractor = ContentExtractor::new();
        let result = extractor.extract_content(&file_path).unwrap();

        assert!(result.content.contains("main"));
        assert!(result.content.contains("println"));
        assert_eq!(result.file_type, FileType::Code);
    }

    #[test]
    fn test_strip_html_tags() {
        let extractor = ContentExtractor::new();
        let html = "<html><body><h1>Title</h1><p>Paragraph</p><script>alert('test');</script></body></html>";
        let text = extractor.strip_html_tags(html);

        assert!(text.contains("Title"));
        assert!(text.contains("Paragraph"));
        assert!(!text.contains("<h1>"));
        assert!(!text.contains("<script>"));
        assert!(!text.contains("alert"));
    }

    #[test]
    fn test_preprocess_text() {
        let extractor = ContentExtractor::new();
        let text = "  Hello  \n\n  World  \r\n";
        let processed = extractor.preprocess_text(text);

        assert_eq!(processed, "Hello World");
    }

    #[test]
    fn test_extract_unsupported_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.jpg");

        fs::write(&file_path, "fake image data").unwrap();

        let extractor = ContentExtractor::new();
        let result = extractor.extract_content(&file_path).unwrap();

        assert_eq!(result.content, "");
        assert_eq!(result.file_type, FileType::Binary);
        assert_eq!(result.content_length, 0);
    }
}
