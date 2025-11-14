//! 文件类型检测
//!
//! 用于根据文件内容检测文件类型，以便采用不同的分块策略

use serde::{Deserialize, Serialize};

/// 文件类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    /// 文本文件（UTF-8编码率高，可打印字符多）
    Text,
    /// 二进制可执行文件
    Binary,
    /// 压缩文件/归档文件
    Archive,
    /// 图像文件
    Image,
    /// 视频文件
    Video,
    /// 音频文件
    Audio,
    /// 未知类型
    Unknown,
}

impl FileType {
    /// 根据文件内容检测类型
    pub fn detect(data: &[u8]) -> Self {
        if data.is_empty() {
            return Self::Unknown;
        }

        // 1. 检查文件头魔数
        if let Some(file_type) = Self::detect_by_magic_bytes(data) {
            return file_type;
        }

        // 2. 检查是否为文本文件
        if Self::is_text(data) {
            return Self::Text;
        }

        // 3. 默认为二进制
        Self::Binary
    }

    /// 根据魔数检测文件类型
    fn detect_by_magic_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }

        // 图像格式
        if data.starts_with(b"\x89PNG") {
            return Some(Self::Image);
        }
        if data.starts_with(b"\xFF\xD8\xFF") {
            return Some(Self::Image); // JPEG
        }
        if data.starts_with(b"GIF87a") || data.starts_with(b"GIF89a") {
            return Some(Self::Image);
        }
        if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WEBP" {
            return Some(Self::Image);
        }

        // 压缩/归档格式
        if data.starts_with(b"PK\x03\x04") || data.starts_with(b"PK\x05\x06") {
            return Some(Self::Archive); // ZIP
        }
        if data.starts_with(b"\x1f\x8b") {
            return Some(Self::Archive); // GZIP
        }
        if data.starts_with(b"BZh") {
            return Some(Self::Archive); // BZIP2
        }
        if data.starts_with(b"\xFD7zXZ\x00") {
            return Some(Self::Archive); // XZ
        }
        if data.starts_with(b"Rar!\x1a\x07") {
            return Some(Self::Archive); // RAR
        }
        if data.starts_with(b"7z\xBC\xAF\x27\x1C") {
            return Some(Self::Archive); // 7Z
        }

        // 视频格式
        if data.len() > 12 && &data[4..12] == b"ftypmp42" {
            return Some(Self::Video); // MP4
        }
        if data.starts_with(b"\x00\x00\x00\x18ftypmp42") || data.starts_with(b"\x00\x00\x00\x20ftypisom")
        {
            return Some(Self::Video);
        }
        if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"AVI " {
            return Some(Self::Video);
        }

        // 音频格式
        if data.starts_with(b"ID3") || data.starts_with(b"\xFF\xFB") {
            return Some(Self::Audio); // MP3
        }
        if data.starts_with(b"RIFF") && data.len() > 12 && &data[8..12] == b"WAVE" {
            return Some(Self::Audio);
        }
        if data.starts_with(b"fLaC") {
            return Some(Self::Audio); // FLAC
        }

        None
    }

    /// 检查是否为文本文件
    fn is_text(data: &[u8]) -> bool {
        let sample_size = data.len().min(8192); // 检查前8KB
        let sample = &data[..sample_size];

        // 统计可打印字符和控制字符
        let mut printable = 0;
        let mut control = 0;
        let mut utf8_valid = true;

        for &byte in sample {
            if byte == b'\n' || byte == b'\r' || byte == b'\t' || (32..127).contains(&byte) {
                printable += 1;
            } else if byte < 32 || byte == 127 {
                control += 1;
            }
        }

        // 检查 UTF-8 有效性
        if std::str::from_utf8(sample).is_err() {
            utf8_valid = false;
        }

        // 判断标准：
        // 1. UTF-8 有效
        // 2. 可打印字符占比 > 90%
        // 3. 控制字符占比 < 5%
        let total = sample.len() as f64;
        let printable_ratio = printable as f64 / total;
        let control_ratio = control as f64 / total;

        utf8_valid && printable_ratio > 0.9 && control_ratio < 0.05
    }

    /// 获取推荐的块大小范围 (min, max)
    pub fn recommended_chunk_size(&self) -> (usize, usize) {
        match self {
            Self::Text => (2 * 1024, 8 * 1024),        // 2KB - 8KB，文本去重效果好
            Self::Binary => (4 * 1024, 16 * 1024),     // 4KB - 16KB，标准大小
            Self::Archive => (8 * 1024, 32 * 1024),    // 8KB - 32KB，已压缩，大块减少开销
            Self::Image => (16 * 1024, 64 * 1024),     // 16KB - 64KB，多媒体大块
            Self::Video => (32 * 1024, 128 * 1024),    // 32KB - 128KB，视频大块
            Self::Audio => (16 * 1024, 64 * 1024),     // 16KB - 64KB，音频大块
            Self::Unknown => (4 * 1024, 16 * 1024),    // 4KB - 16KB，默认值
        }
    }

    /// 是否已压缩（不需要再压缩）
    pub fn is_compressed(&self) -> bool {
        matches!(
            self,
            Self::Archive | Self::Image | Self::Video | Self::Audio
        )
    }

    /// 转换为字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Binary => "binary",
            Self::Archive => "archive",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Unknown => "unknown",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_text() {
        let text = b"Hello, World! This is a text file.\nWith multiple lines.";
        assert_eq!(FileType::detect(text), FileType::Text);
    }

    #[test]
    fn test_detect_png() {
        let png_header = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert_eq!(FileType::detect(png_header), FileType::Image);
    }

    #[test]
    fn test_detect_jpeg() {
        let jpeg_header = b"\xFF\xD8\xFF\xE0\x00\x10JFIF";
        assert_eq!(FileType::detect(jpeg_header), FileType::Image);
    }

    #[test]
    fn test_detect_zip() {
        let zip_header = b"PK\x03\x04\x14\x00\x00\x00";
        assert_eq!(FileType::detect(zip_header), FileType::Archive);
    }

    #[test]
    fn test_detect_gzip() {
        let gzip_header = b"\x1f\x8b\x08\x00\x00\x00\x00\x00";
        assert_eq!(FileType::detect(gzip_header), FileType::Archive);
    }

    #[test]
    fn test_detect_mp3() {
        let mp3_header = b"ID3\x03\x00\x00\x00";
        assert_eq!(FileType::detect(mp3_header), FileType::Audio);
    }

    #[test]
    fn test_detect_binary() {
        let binary = vec![0x00, 0x01, 0x02, 0xFF, 0xFE, 0xFD];
        assert_eq!(FileType::detect(&binary), FileType::Binary);
    }

    #[test]
    fn test_recommended_chunk_size() {
        assert_eq!(FileType::Text.recommended_chunk_size(), (2 * 1024, 8 * 1024));
        assert_eq!(
            FileType::Video.recommended_chunk_size(),
            (32 * 1024, 128 * 1024)
        );
    }

    #[test]
    fn test_is_compressed() {
        assert!(!FileType::Text.is_compressed());
        assert!(!FileType::Binary.is_compressed());
        assert!(FileType::Archive.is_compressed());
        assert!(FileType::Image.is_compressed());
        assert!(FileType::Video.is_compressed());
    }

    #[test]
    fn test_as_str() {
        assert_eq!(FileType::Text.as_str(), "text");
        assert_eq!(FileType::Image.as_str(), "image");
        assert_eq!(FileType::Unknown.as_str(), "unknown");
    }
}
