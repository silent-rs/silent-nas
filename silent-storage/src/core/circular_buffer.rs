//! 环形缓冲区实现
//!
//! 用于高效的滑动窗口操作，O(1) 时间复杂度的 push/pop

use std::collections::VecDeque;

/// 环形缓冲区
///
/// 专为滑动窗口设计的高效数据结构
#[derive(Debug, Clone)]
pub struct CircularBuffer {
    /// 内部使用 VecDeque 实现 O(1) 的头尾操作
    buffer: VecDeque<u8>,
    /// 容量限制
    capacity: usize,
}

impl CircularBuffer {
    /// 创建指定容量的环形缓冲区
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// 添加元素，如果已满则移除最旧元素
    ///
    /// 返回被移除的最旧元素（如果有）
    pub fn push(&mut self, byte: u8) -> Option<u8> {
        let removed = if self.buffer.len() == self.capacity {
            self.buffer.pop_front()
        } else {
            None
        };

        self.buffer.push_back(byte);
        removed
    }

    /// 获取最旧元素（不移除）
    pub fn oldest(&self) -> Option<u8> {
        self.buffer.front().copied()
    }

    /// 获取最新元素（不移除）
    #[allow(dead_code)]
    pub fn newest(&self) -> Option<u8> {
        self.buffer.back().copied()
    }

    /// 当前长度
    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    /// 是否为空
    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    /// 是否已满
    pub fn is_full(&self) -> bool {
        self.buffer.len() == self.capacity
    }

    /// 清空缓冲区
    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    /// 获取所有数据的切片视图（用于哈希计算）
    pub fn as_slice(&self) -> Vec<u8> {
        self.buffer.iter().copied().collect()
    }

    /// 容量
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// 从切片初始化缓冲区
    pub fn from_slice(&mut self, data: &[u8]) {
        self.buffer.clear();
        let len = data.len().min(self.capacity);
        self.buffer.extend(&data[..len]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_buffer_basic() {
        let mut buf = CircularBuffer::new(3);

        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());

        // 添加元素
        assert_eq!(buf.push(1), None);
        assert_eq!(buf.push(2), None);
        assert_eq!(buf.push(3), None);

        assert_eq!(buf.len(), 3);
        assert!(buf.is_full());
    }

    #[test]
    fn test_circular_buffer_overflow() {
        let mut buf = CircularBuffer::new(3);

        buf.push(1);
        buf.push(2);
        buf.push(3);

        // 缓冲区已满，添加新元素会移除最旧的
        assert_eq!(buf.push(4), Some(1));
        assert_eq!(buf.oldest(), Some(2));

        assert_eq!(buf.push(5), Some(2));
        assert_eq!(buf.oldest(), Some(3));
    }

    #[test]
    fn test_circular_buffer_as_slice() {
        let mut buf = CircularBuffer::new(3);

        buf.push(1);
        buf.push(2);
        buf.push(3);

        let slice = buf.as_slice();
        assert_eq!(slice, vec![1, 2, 3]);

        buf.push(4);
        let slice = buf.as_slice();
        assert_eq!(slice, vec![2, 3, 4]);
    }

    #[test]
    fn test_circular_buffer_from_slice() {
        let mut buf = CircularBuffer::new(5);
        let data = [1, 2, 3];

        buf.from_slice(&data);
        assert_eq!(buf.len(), 3);
        assert_eq!(buf.as_slice(), vec![1, 2, 3]);

        // 测试超过容量的情况
        let data_large = [1, 2, 3, 4, 5, 6, 7];
        buf.from_slice(&data_large);
        assert_eq!(buf.len(), 5);
        assert_eq!(buf.as_slice(), vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_circular_buffer_clear() {
        let mut buf = CircularBuffer::new(3);

        buf.push(1);
        buf.push(2);
        buf.push(3);

        buf.clear();
        assert_eq!(buf.len(), 0);
        assert!(buf.is_empty());
    }

    #[test]
    fn test_circular_buffer_edge_cases() {
        // 容量为 1 的缓冲区
        let mut buf = CircularBuffer::new(1);
        assert_eq!(buf.push(1), None);
        assert_eq!(buf.push(2), Some(1));
        assert_eq!(buf.oldest(), Some(2));

        // 容量为 0 的缓冲区（虽然不实用）
        let mut buf = CircularBuffer::new(0);
        assert_eq!(buf.push(1), None);
        assert_eq!(buf.len(), 1); // VecDeque 允许超过初始容量
    }
}
