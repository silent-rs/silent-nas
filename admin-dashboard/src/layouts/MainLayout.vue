<template>
  <div class="main-layout">
    <div class="layout-header">
      <div class="header-left">
        <h1>Silent-NAS 管理控制台</h1>
      </div>
      <div class="header-right">
        <el-menu mode="horizontal" :default-active="activeMenu" @select="handleMenuSelect" class="header-menu">
          <el-menu-item index="/dashboard">
            <el-icon><House /></el-icon>
            <span>仪表盘</span>
          </el-menu-item>
          <el-menu-item index="/users">
            <el-icon><User /></el-icon>
            <span>用户管理</span>
          </el-menu-item>
          <el-menu-item index="/s3-keys">
            <el-icon><Key /></el-icon>
            <span>S3密钥</span>
          </el-menu-item>
        </el-menu>
        <div class="user-info" v-if="authStore.user">
          <span>{{ authStore.user.username }}（{{ authStore.user.role }}）</span>
          <el-button type="text" @click="handleLogout" style="margin-left: 10px">退出</el-button>
        </div>
      </div>
    </div>
    <div class="layout-content">
      <router-view />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { House, User, Key } from '@element-plus/icons-vue'
import { useAuthStore } from '@/store/modules/auth'

const router = useRouter()
const route = useRoute()
const authStore = useAuthStore()

// 当前激活的菜单
const activeMenu = computed(() => route.path)

// 菜单选择处理
const handleMenuSelect = (index: string) => {
  router.push(index)
}

// 退出登录
const handleLogout = async () => {
  try {
    await ElMessageBox.confirm('确定要退出登录吗？', '提示', {
      confirmButtonText: '确定',
      cancelButtonText: '取消',
      type: 'warning',
    })
    authStore.logout()
    router.push('/login')
    ElMessage.success('已退出登录')
  } catch (error) {
    // 用户取消操作
  }
}
</script>

<style scoped lang="scss">
.main-layout {
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: #f0f2f5;
}

.layout-header {
  background: #fff;
  box-shadow: 0 2px 4px rgba(0, 0, 0, 0.08);
  padding: 0 24px;
  display: flex;
  justify-content: space-between;
  align-items: center;
  height: 60px;

  .header-left {
    h1 {
      font-size: 20px;
      font-weight: 600;
      color: #303133;
      margin: 0;
    }
  }

  .header-right {
    display: flex;
    align-items: center;
    gap: 20px;

    .header-menu {
      border: none;
      background: transparent;
    }
  }

  .user-info {
    display: flex;
    align-items: center;
    font-size: 14px;
    color: #606266;
    white-space: nowrap;
  }
}

.layout-content {
  flex: 1;
  overflow-y: auto;
  padding: 24px;
}
</style>
