// 用户角色
export const UserRole = {
  Admin: 'Admin',
  User: 'User',
  ReadOnly: 'ReadOnly',
} as const

export type UserRole = typeof UserRole[keyof typeof UserRole]

// 用户信息
export interface User {
  id: string
  username: string
  email?: string
  role: UserRole
  status?: string
  created_at: number
}

// 登录表单
export interface LoginForm {
  username: string
  password: string
}

// 登录响应
export interface LoginResponse {
  access_token: string
  refresh_token: string
  expires_in: number
  token_type: string
  user: User
}

// API 响应
export interface ApiResponse<T = any> {
  code: number
  message: string
  data: T
}
