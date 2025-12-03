//! 管理端 API 模块

pub mod dashboard;

// 重新导出处理器
pub use dashboard::{get_activities, get_metrics, get_overview};
