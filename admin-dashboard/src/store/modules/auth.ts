import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import type { User, LoginForm } from '@/types'
import { login as loginApi, logout as logoutApi, getCurrentUser } from '@/api/auth'

export const useAuthStore = defineStore('auth', () => {
  const token = ref<string>(localStorage.getItem('token') || '')
  const user = ref<User | null>(null)

  const isAuthenticated = computed(() => !!token.value)

  async function login(loginForm: LoginForm) {
    try {
      const res = await loginApi(loginForm)
      token.value = res.access_token
      user.value = res.user
      localStorage.setItem('token', res.access_token)
      localStorage.setItem('refresh_token', res.refresh_token)
      return true
    } catch (error) {
      console.error('Login failed:', error)
      return false
    }
  }

  async function logout() {
    try {
      await logoutApi()
    } catch (error) {
      console.error('Logout failed:', error)
    } finally {
      token.value = ''
      user.value = null
      localStorage.removeItem('token')
    }
  }

  async function fetchCurrentUser() {
    if (!token.value) return

    try {
      const userData = await getCurrentUser()
      user.value = userData
    } catch (error) {
      console.error('Failed to fetch current user:', error)
      // Token 可能已过期，清除登录状态
      logout()
    }
  }

  return {
    token,
    user,
    isAuthenticated,
    login,
    logout,
    fetchCurrentUser,
  }
})
