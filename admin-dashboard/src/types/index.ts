// 用户角色
export enum UserRole {
  Admin = 'Admin',
  User = 'User',
  ReadOnly = 'ReadOnly',
}

// 用户信息
export interface User {
  id: string
  username: string
  role: UserRole
  created_at: string
}

// 登录表单
export interface LoginForm {
  username: string
  password: string
}

// 登录响应
export interface LoginResponse {
  token: string
  user: User
}

// API 响应
export interface ApiResponse<T = any> {
  code: number
  message: string
  data: T
}
