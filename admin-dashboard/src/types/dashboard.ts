// 存储使用情况
export interface StorageUsage {
  used_bytes: number
  total_bytes: number
  available_bytes: number
  usage_percent: number
}

// 系统概览
export interface SystemOverview {
  file_count: number
  user_count: number
  storage: StorageUsage
  online_nodes: number
}
