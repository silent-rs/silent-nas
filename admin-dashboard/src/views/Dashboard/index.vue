<template>
  <div class="dashboard-container">
    <div class="dashboard-header">
      <h1>欢迎使用 Silent-NAS 管理控制台</h1>
      <p v-if="authStore.user">当前用户：{{ authStore.user.username }}（{{ authStore.user.role }}）</p>
    </div>

    <div class="dashboard-content">
      <el-row :gutter="20">
        <el-col :span="6">
          <el-card shadow="hover">
            <template #header>
              <div class="card-header">
                <el-icon><Folder /></el-icon>
                <span>文件总数</span>
              </div>
            </template>
            <div class="stat-value">--</div>
            <div class="stat-label">暂无数据</div>
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
            <div class="stat-value">--</div>
            <div class="stat-label">暂无数据</div>
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
            <div class="stat-value">--</div>
            <div class="stat-label">暂无数据</div>
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
            <div class="stat-value">--</div>
            <div class="stat-label">暂无数据</div>
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
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { useAuthStore } from '@/store/modules/auth'

const router = useRouter()
const authStore = useAuthStore()

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
