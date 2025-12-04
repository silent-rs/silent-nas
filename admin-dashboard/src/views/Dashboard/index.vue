<template>
  <div class="dashboard-container">

    <div class="dashboard-content">
      <el-row :gutter="20" v-loading="loading">
        <el-col :span="6">
          <el-card shadow="hover" class="clickable-card" @click="showFileExplorer">
            <template #header>
              <div class="card-header">
                <el-icon><Folder /></el-icon>
                <span>文件总数</span>
              </div>
            </template>
            <div class="stat-value">{{ overview?.file_count ?? '--' }}</div>
            <div class="stat-label">个文件 (点击查看)</div>
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
                <el-icon><Odometer /></el-icon>
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
              <el-button type="primary" @click="() => router.push('/users')">用户管理</el-button>
            </div>
          </el-card>
        </el-col>
      </el-row>
    </div>

    <!-- 文件浏览器对话框 -->
    <FileExplorer v-model="fileExplorerVisible" @refresh="loadOverview" />
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { User, Folder, Odometer, Link, Operation } from '@element-plus/icons-vue'
import { getSystemOverview } from '@/api/dashboard'
import type { SystemOverview } from '@/types/dashboard'
import FileExplorer from '@/components/FileExplorer.vue'

const router = useRouter()

const loading = ref(true)
const overview = ref<SystemOverview | null>(null)
const fileExplorerVisible = ref(false)

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

// 显示文件浏览器
const showFileExplorer = () => {
  fileExplorerVisible.value = true
}

// 组件挂载时加载数据
onMounted(() => {
  loadOverview()
})
</script>

<style scoped lang="scss">
.dashboard-container {
  height: 100%;
}

.dashboard-content {
  .clickable-card {
    cursor: pointer;
    transition: transform 0.2s, box-shadow 0.2s;

    &:hover {
      transform: translateY(-2px);
      box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
    }
  }

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
