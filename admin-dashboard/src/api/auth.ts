import request from '@/utils/request'
import type { LoginForm, LoginResponse, User } from '@/types'

// 用户登录
export function login(data: LoginForm): Promise<LoginResponse> {
  return request({
    url: '/auth/login',
    method: 'post',
    data,
  })
}

// 用户登出
export function logout(): Promise<void> {
  return request({
    url: '/auth/logout',
    method: 'post',
  })
}

// 获取当前用户信息
export function getCurrentUser(): Promise<User> {
  return request({
    url: '/auth/me',
    method: 'get',
  })
}

// 刷新 Token
export function refreshToken(): Promise<{ token: string }> {
  return request({
    url: '/auth/refresh',
    method: 'post',
  })
}
