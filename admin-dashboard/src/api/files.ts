import request from '@/utils/request'
import type { FileItem } from '@/types/files'

// 获取文件列表
export function getFileList(): Promise<FileItem[]> {
  return request({
    url: '/files',
    method: 'get',
  })
}

// 下载文件
export function downloadFile(fileId: string): string {
  return `/api/files/${fileId}`
}

// 删除文件
export function deleteFile(fileId: string): Promise<void> {
  return request({
    url: `/files/${fileId}`,
    method: 'delete',
  })
}
