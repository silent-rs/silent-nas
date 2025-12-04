<template>
  <div class="s3-keys-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>S3 访问密钥管理</span>
          <el-button type="primary" @click="handleCreate">创建密钥</el-button>
        </div>
      </template>

      <!-- 搜索栏 -->
      <div class="search-bar">
        <el-input
          v-model="searchText"
          placeholder="搜索访问密钥或描述"
          style="width: 300px"
          clearable
          @input="handleSearch"
        >
          <template #prefix>
            <el-icon><Search /></el-icon>
          </template>
        </el-input>
        <el-select v-model="statusFilter" placeholder="状态筛选" clearable style="width: 150px; margin-left: 10px">
          <el-option label="全部状态" value="" />
          <el-option label="活跃" value="Active" />
          <el-option label="已禁用" value="Disabled" />
        </el-select>
      </div>

      <!-- 密钥列表 -->
      <el-table :data="filteredKeys" v-loading="loading" style="width: 100%; margin-top: 20px">
        <el-table-column prop="access_key" label="访问密钥" width="220">
          <template #default="{ row }">
            <code>{{ row.access_key }}</code>
          </template>
        </el-table-column>
        <el-table-column prop="description" label="描述" min-width="200" />
        <el-table-column prop="status" label="状态" width="100">
          <template #default="{ row }">
            <el-tag :type="getStatusTagType(row.status)">
              {{ getStatusLabel(row.status) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="created_at" label="创建时间" width="180">
          <template #default="{ row }">
            {{ formatTime(row.created_at) }}
          </template>
        </el-table-column>
        <el-table-column prop="last_used_at" label="最后使用" width="180">
          <template #default="{ row }">
            {{ row.last_used_at ? formatTime(row.last_used_at) : '从未使用' }}
          </template>
        </el-table-column>
        <el-table-column label="操作" fixed="right" width="200">
          <template #default="{ row }">
            <el-button type="primary" size="small" @click="handleEdit(row)">编辑</el-button>
            <el-button
              :type="row.status === 'Active' ? 'warning' : 'success'"
              size="small"
              @click="handleToggleStatus(row)"
            >
              {{ row.status === 'Active' ? '禁用' : '启用' }}
            </el-button>
            <el-button type="danger" size="small" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 创建密钥对话框 -->
    <el-dialog v-model="createDialogVisible" title="创建 S3 访问密钥" width="500px">
      <el-form :model="createForm" :rules="createRules" ref="createFormRef" label-width="80px">
        <el-form-item label="描述" prop="description">
          <el-input v-model="createForm.description" placeholder="请输入密钥描述" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmitCreate" :loading="submitting">创建</el-button>
      </template>
    </el-dialog>

    <!-- 密钥创建成功对话框（显示密钥密钥）-->
    <el-dialog v-model="keyCreatedDialogVisible" title="密钥创建成功" width="600px" :close-on-click-modal="false">
      <el-alert
        title="重要提示"
        type="warning"
        description="这是您唯一一次可以看到密钥密钥（Secret Key）的机会，请妥善保存。关闭此对话框后将无法再次查看。"
        :closable="false"
        show-icon
        style="margin-bottom: 20px"
      />
      <el-form label-width="100px">
        <el-form-item label="访问密钥">
          <el-input v-model="createdKey.access_key" readonly>
            <template #append>
              <el-button @click="copyToClipboard(createdKey.access_key)">复制</el-button>
            </template>
          </el-input>
        </el-form-item>
        <el-form-item label="密钥密钥">
          <el-input v-model="createdKey.secret_key" readonly type="textarea" :rows="3">
          </el-input>
          <el-button @click="copyToClipboard(createdKey.secret_key)" style="margin-top: 10px">复制密钥密钥</el-button>
        </el-form-item>
        <el-form-item label="描述">
          <span>{{ createdKey.description }}</span>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button type="primary" @click="handleCloseCreatedDialog">我已保存密钥</el-button>
      </template>
    </el-dialog>

    <!-- 编辑密钥对话框 -->
    <el-dialog v-model="editDialogVisible" title="编辑 S3 访问密钥" width="500px">
      <el-form :model="editForm" :rules="editRules" ref="editFormRef" label-width="80px">
        <el-form-item label="访问密钥">
          <code>{{ editForm.access_key }}</code>
        </el-form-item>
        <el-form-item label="描述" prop="description">
          <el-input v-model="editForm.description" placeholder="请输入密钥描述" />
        </el-form-item>
        <el-form-item label="状态" prop="status">
          <el-select v-model="editForm.status" placeholder="请选择状态" style="width: 100%">
            <el-option label="活跃" value="Active" />
            <el-option label="已禁用" value="Disabled" />
          </el-select>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="editDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmitEdit" :loading="submitting">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { Search } from '@element-plus/icons-vue'
import { listS3Keys, createS3Key, updateS3Key, deleteS3Key } from '@/api/s3Keys'
import type { S3AccessKey, S3KeyStatus, CreateS3KeyResponse } from '@/types/s3Keys'

// 数据状态
const loading = ref(false)
const keys = ref<S3AccessKey[]>([])
const searchText = ref('')
const statusFilter = ref('')

// 对话框状态
const createDialogVisible = ref(false)
const keyCreatedDialogVisible = ref(false)
const editDialogVisible = ref(false)
const submitting = ref(false)

// 表单数据
const createForm = ref({
  description: '',
})

const createdKey = ref({
  access_key: '',
  secret_key: '',
  description: '',
})

const editForm = ref({
  id: '',
  access_key: '',
  description: '',
  status: 'Active' as S3KeyStatus,
})

// 表单引用
const createFormRef = ref<FormInstance>()
const editFormRef = ref<FormInstance>()

// 表单验证规则
const createRules: FormRules = {
  description: [
    { required: true, message: '请输入密钥描述', trigger: 'blur' },
    { max: 200, message: '描述长度不能超过 200 个字符', trigger: 'blur' },
  ],
}

const editRules: FormRules = {
  description: [
    { required: true, message: '请输入密钥描述', trigger: 'blur' },
    { max: 200, message: '描述长度不能超过 200 个字符', trigger: 'blur' },
  ],
}

// 过滤后的密钥列表
const filteredKeys = computed(() => {
  let result = keys.value

  // 文本搜索
  if (searchText.value) {
    const text = searchText.value.toLowerCase()
    result = result.filter(
      (key) =>
        key.access_key.toLowerCase().includes(text) ||
        key.description.toLowerCase().includes(text)
    )
  }

  // 状态筛选
  if (statusFilter.value) {
    result = result.filter((key) => key.status === statusFilter.value)
  }

  return result
})

// 加载密钥列表
const loadKeys = async () => {
  try {
    loading.value = true
    keys.value = await listS3Keys()
  } catch (error) {
    ElMessage.error('加载 S3 密钥列表失败')
    console.error(error)
  } finally {
    loading.value = false
  }
}

// 搜索处理
const handleSearch = () => {
  // 搜索逻辑在 computed 中自动处理
}

// 创建密钥
const handleCreate = () => {
  createForm.value = {
    description: '',
  }
  createDialogVisible.value = true
}

// 提交创建
const handleSubmitCreate = async () => {
  if (!createFormRef.value) return

  await createFormRef.value.validate(async (valid) => {
    if (!valid) return

    try {
      submitting.value = true

      const response: CreateS3KeyResponse = await createS3Key({
        description: createForm.value.description,
      })

      createdKey.value = {
        access_key: response.access_key,
        secret_key: response.secret_key,
        description: response.description,
      }

      createDialogVisible.value = false
      keyCreatedDialogVisible.value = true

      await loadKeys()
    } catch (error) {
      ElMessage.error('创建 S3 密钥失败')
      console.error(error)
    } finally {
      submitting.value = false
    }
  })
}

// 关闭密钥创建成功对话框
const handleCloseCreatedDialog = () => {
  keyCreatedDialogVisible.value = false
  createdKey.value = {
    access_key: '',
    secret_key: '',
    description: '',
  }
}

// 编辑密钥
const handleEdit = (key: S3AccessKey) => {
  editForm.value = {
    id: key.id,
    access_key: key.access_key,
    description: key.description,
    status: key.status,
  }
  editDialogVisible.value = true
}

// 提交编辑
const handleSubmitEdit = async () => {
  if (!editFormRef.value) return

  await editFormRef.value.validate(async (valid) => {
    if (!valid) return

    try {
      submitting.value = true

      await updateS3Key(editForm.value.id, {
        description: editForm.value.description,
        status: editForm.value.status,
      })

      ElMessage.success('更新 S3 密钥成功')
      editDialogVisible.value = false
      await loadKeys()
    } catch (error) {
      ElMessage.error('更新 S3 密钥失败')
      console.error(error)
    } finally {
      submitting.value = false
    }
  })
}

// 切换密钥状态
const handleToggleStatus = async (key: S3AccessKey) => {
  const newStatus = key.status === 'Active' ? 'Disabled' : 'Active'
  const action = newStatus === 'Active' ? '启用' : '禁用'

  try {
    await ElMessageBox.confirm(`确定要${action}密钥"${key.access_key}"吗？`, '确认操作', {
      confirmButtonText: '确定',
      cancelButtonText: '取消',
      type: 'warning',
    })

    await updateS3Key(key.id, { status: newStatus })
    ElMessage.success(`${action}密钥成功`)
    await loadKeys()
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error(`${action}密钥失败`)
      console.error(error)
    }
  }
}

// 删除密钥
const handleDelete = async (key: S3AccessKey) => {
  try {
    await ElMessageBox.confirm(
      `确定要删除密钥"${key.access_key}"吗？此操作不可恢复。`,
      '确认删除',
      {
        confirmButtonText: '确定',
        cancelButtonText: '取消',
        type: 'warning',
      }
    )

    await deleteS3Key(key.id)
    ElMessage.success('删除 S3 密钥成功')
    await loadKeys()
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error('删除 S3 密钥失败')
      console.error(error)
    }
  }
}

// 复制到剪贴板
const copyToClipboard = async (text: string) => {
  try {
    await navigator.clipboard.writeText(text)
    ElMessage.success('已复制到剪贴板')
  } catch (error) {
    ElMessage.error('复制失败')
    console.error(error)
  }
}

// 获取状态标签类型
const getStatusTagType = (status: string) => {
  const types: Record<string, any> = {
    Active: 'success',
    Disabled: 'info',
  }
  return types[status] || 'info'
}

// 获取状态标签文本
const getStatusLabel = (status: string) => {
  const labels: Record<string, string> = {
    Active: '活跃',
    Disabled: '已禁用',
  }
  return labels[status] || status
}

// 格式化时间
const formatTime = (timestamp: number) => {
  const date = new Date(timestamp * 1000)
  return date.toLocaleString('zh-CN', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

// 组件挂载时加载数据
onMounted(() => {
  loadKeys()
})
</script>

<style scoped lang="scss">
.s3-keys-container {
  height: 100%;

  .card-header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }

  .search-bar {
    display: flex;
    align-items: center;
  }

  code {
    background-color: #f5f7fa;
    padding: 2px 6px;
    border-radius: 3px;
    font-family: 'Courier New', Courier, monospace;
    font-size: 13px;
  }
}
</style>
