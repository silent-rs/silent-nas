import request from '@/utils/request'
import type {
  S3AccessKey,
  CreateS3KeyRequest,
  CreateS3KeyResponse,
  UpdateS3KeyRequest,
} from '@/types/s3Keys'

export function listS3Keys(): Promise<S3AccessKey[]> {
  return request({
    url: '/admin/s3-keys',
    method: 'get',
  })
}

export function listAllS3Keys(): Promise<S3AccessKey[]> {
  return request({
    url: '/admin/s3-keys/all',
    method: 'get',
  })
}

export function getS3Key(id: string): Promise<S3AccessKey> {
  return request({
    url: `/admin/s3-keys/${id}`,
    method: 'get',
  })
}

export function createS3Key(data: CreateS3KeyRequest): Promise<CreateS3KeyResponse> {
  return request({
    url: '/admin/s3-keys',
    method: 'post',
    data,
  })
}

export function updateS3Key(id: string, data: UpdateS3KeyRequest): Promise<S3AccessKey> {
  return request({
    url: `/admin/s3-keys/${id}`,
    method: 'put',
    data,
  })
}

export function deleteS3Key(id: string): Promise<void> {
  return request({
    url: `/admin/s3-keys/${id}`,
    method: 'delete',
  })
}
