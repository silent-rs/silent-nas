import request from '@/utils/request'
import type { User, CreateUserRequest, UpdateUserRequest, ChangePasswordRequest } from '@/types/user'

/**
 * 获取用户列表
 */
export function getUserList(): Promise<User[]> {
  return request({
    url: '/admin/users',
    method: 'get',
  })
}

/**
 * 获取用户详情
 */
export function getUserDetail(userId: string): Promise<User> {
  return request({
    url: `/admin/users/${userId}`,
    method: 'get',
  })
}

/**
 * 创建用户
 */
export function createUser(data: CreateUserRequest): Promise<User> {
  return request({
    url: '/admin/users',
    method: 'post',
    data,
  })
}

/**
 * 更新用户
 */
export function updateUser(userId: string, data: UpdateUserRequest): Promise<User> {
  return request({
    url: `/admin/users/${userId}`,
    method: 'put',
    data,
  })
}

/**
 * 删除用户
 */
export function deleteUser(userId: string): Promise<void> {
  return request({
    url: `/admin/users/${userId}`,
    method: 'delete',
  })
}

/**
 * 修改用户密码
 */
export function changeUserPassword(data: ChangePasswordRequest): Promise<void> {
  return request({
    url: `/admin/users/${data.user_id}/password`,
    method: 'put',
    data: {
      new_password: data.new_password,
    },
  })
}

/**
 * 修改用户状态
 */
export function changeUserStatus(userId: string, status: string): Promise<void> {
  return request({
    url: `/admin/users/${userId}/status`,
    method: 'put',
    data: {
      status,
    },
  })
}
