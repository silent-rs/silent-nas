export interface S3AccessKey {
  id: string
  user_id: string
  access_key: string
  description: string
  status: S3KeyStatus
  created_at: number
  last_used_at?: number
}

export const S3KeyStatus = {
  Active: 'Active',
  Disabled: 'Disabled',
} as const

export type S3KeyStatus = typeof S3KeyStatus[keyof typeof S3KeyStatus]

export interface CreateS3KeyRequest {
  description: string
}

export interface CreateS3KeyResponse {
  id: string
  access_key: string
  secret_key: string
  description: string
  status: S3KeyStatus
  created_at: number
}

export interface UpdateS3KeyRequest {
  description?: string
  status?: S3KeyStatus
}
