export interface User {
  id: string
  username: string
  email: string
  role: UserRole
  status: UserStatus
  created_at: number
}

export const UserRole = {
  Admin: 'Admin',
  User: 'User',
  ReadOnly: 'ReadOnly',
} as const

export type UserRole = typeof UserRole[keyof typeof UserRole]

export const UserStatus = {
  Active: 'Active',
  Suspended: 'Suspended',
  Deleted: 'Deleted',
} as const

export type UserStatus = typeof UserStatus[keyof typeof UserStatus]

export interface CreateUserRequest {
  username: string
  password: string
  email?: string
  role: UserRole
}

export interface UpdateUserRequest {
  email?: string
  role?: UserRole
  status?: UserStatus
}

export interface ChangePasswordRequest {
  user_id: string
  new_password: string
}
