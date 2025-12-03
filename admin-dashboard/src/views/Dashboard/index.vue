<template>
  <div class="dashboard-container">
    <div class="dashboard-header">
      <h1>欢迎使用 Silent-NAS 管理控制台</h1>
      <p v-if="authStore.user">当前用户：{{ authStore.user.username }}（{{ authStore.user.role }}）</p>
    </div>

    <div class="dashboard-content">
      <el-row :gutter="20" v-loading="loading">
        <el-col :span="6">
          <el-card shadow="hover">
            <template #header>
              <div class="card-header">
                <el-icon><Folder /></el-icon>
                <span>文件总数</span>
              </div>
            </template>
            <div class="stat-value">{{ overview?.file_count ?? '--' }}</div>
            <div class="stat-label">个文件</div>
          </el-card>
        </el-col>

        <el-col :span="6">
          <el-card shadow="hover">
            <template #header>
              <div class="card-header">
                <el-icon><User /></el-icon>
                <span>用户总数</span>
              </div>
            </template>
            <div class="stat-value">{{ overview?.user_count ?? '--' }}</div>
            <div class="stat-label">个用户</div>
          </el-card>
        </el-col>

        <el-col :span="6">
          <el-card shadow="hover">
            <template #header>
              <div class="card-header">
                <el-icon><HardDrive /></el-icon>
                <span>存储使用</span>
              </div>
            </template>
            <div class="stat-value">
              {{ overview?.storage ? formatBytes(overview.storage.used_bytes) : '--' }}
            </div>
            <div class="stat-label">
              已用 / 总计 {{ overview?.storage ? formatBytes(overview.storage.total_bytes) : '--' }}
              <div v-if="overview?.storage" style="margin-top: 4px">
                <el-progress
                  :percentage="overview.storage.usage_percent * 100"
                  :stroke-width="6"
                  :show-text="false"
                />
              </div>
            </div>
          </el-card>
        </el-col>

        <el-col :span="6">
          <el-card shadow="hover">
            <template #header>
              <div class="card-header">
                <el-icon><Link /></el-icon>
                <span>在线节点</span>
              </div>
            </template>
            <div class="stat-value">{{ overview?.online_nodes ?? '--' }}</div>
            <div class="stat-label">个节点在线</div>
          </el-card>
        </el-col>
      </el-row>

      <el-row :gutter="20" style="margin-top: 20px">
        <el-col :span="24">
          <el-card>
            <template #header>
              <div class="card-header">
                <el-icon><Operation /></el-icon>
                <span>快捷操作</span>
              </div>
            </template>
            <div class="actions">
              <el-button type="primary" @click="handleLogout">退出登录</el-button>
            </div>
          </el-card>
        </el-col>
      </el-row>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAuthStore } from '@/store/modules/auth'
import { getSystemOverview } from '@/api/dashboard'
import type { SystemOverview } from '@/types/dashboard'

const router = useRouter()
const authStore = useAuthStore()

const loading = ref(true)
const overview = ref<SystemOverview | null>(null)

// 格式化字节数为可读格式
const formatBytes = (bytes: number): string => {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return Math.round(bytes / Math.pow(k, i) * 100) / 100 + ' ' + sizes[i]
}

// 加载系统概览数据
const loadOverview = async () => {
  try {
    loading.value = true
    overview.value = await getSystemOverview()
  } catch (error) {
    console.error('Failed to load overview:', error)
    ElMessage.error('加载系统数据失败')
  } finally {
    loading.value = false
  }
}

const handleLogout = async () => {
  try {
    await ElMessageBox.confirm('确定要退出登录吗？', '提示', {
      confirmButtonText: '确定',
      cancelButtonText: '取消',
      type: 'warning',
    })

    await authStore.logout()
    ElMessage.success('已退出登录')
    router.push('/login')
  } catch (error) {
    // 用户取消操作
  }
}

// 组件挂载时加载数据
onMounted(() => {
  loadOverview()
})
</script>

<style scoped lang="scss">
.dashboard-container {
  padding: 24px;
  height: 100%;
  overflow-y: auto;
}

.dashboard-header {
  margin-bottom: 24px;

  h1 {
    font-size: 24px;
    font-weight: 600;
    color: #303133;
    margin-bottom: 8px;
  }

  p {
    font-size: 14px;
    color: #909399;
  }
}

.dashboard-content {
  .card-header {
    display: flex;
    align-items: center;
    gap: 8px;
    font-weight: 600;
  }

  .stat-value {
    font-size: 32px;
    font-weight: 600;
    color: #409eff;
    margin-bottom: 8px;
  }

  .stat-label {
    font-size: 14px;
    color: #909399;
  }

  .actions {
    display: flex;
    gap: 12px;
  }
}
</style>
