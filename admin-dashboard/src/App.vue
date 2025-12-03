<template>
  <router-view />
</template>

<script setup lang="ts">
import { onMounted } from 'vue'
import { useAuthStore } from '@/store/modules/auth'

const authStore = useAuthStore()

onMounted(() => {
  // 如果已登录，获取当前用户信息
  if (authStore.isAuthenticated) {
    authStore.fetchCurrentUser()
  }
})
</script>

<style>
/* 全局样式已在 global.scss 中定义 */
</style>
