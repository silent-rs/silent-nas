import request from '@/utils/request'
import type { SystemOverview } from '@/types/dashboard'

// 获取系统概览
export function getSystemOverview(): Promise<SystemOverview> {
  return request({
    url: '/admin/dashboard/overview',
    method: 'get',
  })
}

// 获取系统指标
export function getSystemMetrics(): Promise<any> {
  return request({
    url: '/admin/dashboard/metrics',
    method: 'get',
  })
}

// 获取最近活动
export function getRecentActivities(): Promise<any> {
  return request({
    url: '/admin/dashboard/activities',
    method: 'get',
  })
}
