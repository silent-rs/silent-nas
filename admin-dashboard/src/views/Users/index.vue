<template>
  <div class="users-container">
    <el-card>
      <template #header>
        <div class="card-header">
          <span>用户管理</span>
          <el-button type="primary" @click="handleCreate">新建用户</el-button>
        </div>
      </template>

      <!-- 搜索栏 -->
      <div class="search-bar">
        <el-input
          v-model="searchText"
          placeholder="搜索用户名或邮箱"
          style="width: 300px"
          clearable
          @input="handleSearch"
        >
          <template #prefix>
            <el-icon><Search /></el-icon>
          </template>
        </el-input>
        <el-select v-model="roleFilter" placeholder="角色筛选" clearable style="width: 150px; margin-left: 10px">
          <el-option label="全部角色" value="" />
          <el-option label="管理员" value="Admin" />
          <el-option label="普通用户" value="User" />
          <el-option label="只读用户" value="ReadOnly" />
        </el-select>
        <el-select v-model="statusFilter" placeholder="状态筛选" clearable style="width: 150px; margin-left: 10px">
          <el-option label="全部状态" value="" />
          <el-option label="活跃" value="Active" />
          <el-option label="暂停" value="Suspended" />
          <el-option label="已删除" value="Deleted" />
        </el-select>
      </div>

      <!-- 用户列表 -->
      <el-table :data="filteredUsers" v-loading="loading" style="width: 100%; margin-top: 20px">
        <el-table-column prop="username" label="用户名" width="180" />
        <el-table-column prop="email" label="邮箱" width="220" />
        <el-table-column prop="role" label="角色" width="120">
          <template #default="{ row }">
            <el-tag :type="getRoleTagType(row.role)">
              {{ getRoleLabel(row.role) }}
            </el-tag>
          </template>
        </el-table-column>
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
        <el-table-column label="操作" fixed="right" width="280">
          <template #default="{ row }">
            <el-button type="primary" size="small" @click="handleEdit(row)">编辑</el-button>
            <el-button type="warning" size="small" @click="handleChangePassword(row)">改密</el-button>
            <el-button
              :type="row.status === 'Active' ? 'warning' : 'success'"
              size="small"
              @click="handleToggleStatus(row)"
            >
              {{ row.status === 'Active' ? '暂停' : '激活' }}
            </el-button>
            <el-button type="danger" size="small" @click="handleDelete(row)">删除</el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 创建/编辑用户对话框 -->
    <el-dialog
      v-model="dialogVisible"
      :title="dialogMode === 'create' ? '创建用户' : '编辑用户'"
      width="500px"
    >
      <el-form :model="userForm" :rules="formRules" ref="formRef" label-width="80px">
        <el-form-item label="用户名" prop="username" v-if="dialogMode === 'create'">
          <el-input v-model="userForm.username" placeholder="请输入用户名" />
        </el-form-item>
        <el-form-item label="密码" prop="password" v-if="dialogMode === 'create'">
          <el-input v-model="userForm.password" type="password" placeholder="请输入密码" show-password />
        </el-form-item>
        <el-form-item label="邮箱" prop="email">
          <el-input v-model="userForm.email" placeholder="请输入邮箱（可选）" />
        </el-form-item>
        <el-form-item label="角色" prop="role">
          <el-select v-model="userForm.role" placeholder="请选择角色" style="width: 100%">
            <el-option label="管理员" value="Admin" />
            <el-option label="普通用户" value="User" />
            <el-option label="只读用户" value="ReadOnly" />
          </el-select>
        </el-form-item>
        <el-form-item label="状态" prop="status" v-if="dialogMode === 'edit'">
          <el-select v-model="userForm.status" placeholder="请选择状态" style="width: 100%">
            <el-option label="活跃" value="Active" />
            <el-option label="暂停" value="Suspended" />
          </el-select>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="dialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmit" :loading="submitting">确定</el-button>
      </template>
    </el-dialog>

    <!-- 修改密码对话框 -->
    <el-dialog v-model="passwordDialogVisible" title="修改密码" width="400px">
      <el-form :model="passwordForm" :rules="passwordRules" ref="passwordFormRef" label-width="80px">
        <el-form-item label="新密码" prop="new_password">
          <el-input
            v-model="passwordForm.new_password"
            type="password"
            placeholder="请输入新密码"
            show-password
          />
        </el-form-item>
        <el-form-item label="确认密码" prop="confirm_password">
          <el-input
            v-model="passwordForm.confirm_password"
            type="password"
            placeholder="请再次输入新密码"
            show-password
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="passwordDialogVisible = false">取消</el-button>
        <el-button type="primary" @click="handleSubmitPassword" :loading="submitting">确定</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted } from 'vue'
import { ElMessage, ElMessageBox, type FormInstance, type FormRules } from 'element-plus'
import { Search } from '@element-plus/icons-vue'
import { getUserList, createUser, updateUser, deleteUser, changeUserPassword, changeUserStatus } from '@/api/users'
import type { User, UserRole } from '@/types/user'

// 数据状态
const loading = ref(false)
const users = ref<User[]>([])
const searchText = ref('')
const roleFilter = ref('')
const statusFilter = ref('')

// 对话框状态
const dialogVisible = ref(false)
const passwordDialogVisible = ref(false)
const dialogMode = ref<'create' | 'edit'>('create')
const submitting = ref(false)

// 表单数据
const userForm = ref({
  id: '',
  username: '',
  password: '',
  email: '',
  role: 'User' as UserRole,
  status: 'Active',
})

const passwordForm = ref({
  user_id: '',
  new_password: '',
  confirm_password: '',
})

// 表单引用
const formRef = ref<FormInstance>()
const passwordFormRef = ref<FormInstance>()

// 表单验证规则
const formRules: FormRules = {
  username: [
    { required: true, message: '请输入用户名', trigger: 'blur' },
    { min: 3, max: 32, message: '用户名长度应在 3-32 个字符', trigger: 'blur' },
  ],
  password: [
    { required: true, message: '请输入密码', trigger: 'blur' },
    { min: 6, message: '密码长度至少 6 个字符', trigger: 'blur' },
  ],
  email: [
    { type: 'email', message: '请输入正确的邮箱地址', trigger: 'blur' },
  ],
  role: [
    { required: true, message: '请选择角色', trigger: 'change' },
  ],
}

const passwordRules: FormRules = {
  new_password: [
    { required: true, message: '请输入新密码', trigger: 'blur' },
    { min: 6, message: '密码长度至少 6 个字符', trigger: 'blur' },
  ],
  confirm_password: [
    { required: true, message: '请再次输入密码', trigger: 'blur' },
    {
      validator: (_rule, value, callback) => {
        if (value !== passwordForm.value.new_password) {
          callback(new Error('两次输入的密码不一致'))
        } else {
          callback()
        }
      },
      trigger: 'blur',
    },
  ],
}

// 过滤后的用户列表
const filteredUsers = computed(() => {
  let result = users.value

  // 文本搜索
  if (searchText.value) {
    const text = searchText.value.toLowerCase()
    result = result.filter(
      (user) =>
        user.username.toLowerCase().includes(text) ||
        user.email.toLowerCase().includes(text)
    )
  }

  // 角色筛选
  if (roleFilter.value) {
    result = result.filter((user) => user.role === roleFilter.value)
  }

  // 状态筛选
  if (statusFilter.value) {
    result = result.filter((user) => user.status === statusFilter.value)
  }

  return result
})

// 加载用户列表
const loadUsers = async () => {
  try {
    loading.value = true
    users.value = await getUserList()
  } catch (error) {
    ElMessage.error('加载用户列表失败')
    console.error(error)
  } finally {
    loading.value = false
  }
}

// 搜索处理
const handleSearch = () => {
  // 搜索逻辑在 computed 中自动处理
}

// 创建用户
const handleCreate = () => {
  dialogMode.value = 'create'
  userForm.value = {
    id: '',
    username: '',
    password: '',
    email: '',
    role: 'User',
    status: 'Active',
  }
  dialogVisible.value = true
}

// 编辑用户
const handleEdit = (user: User) => {
  dialogMode.value = 'edit'
  userForm.value = {
    id: user.id,
    username: user.username,
    password: '',
    email: user.email,
    role: user.role,
    status: user.status,
  }
  dialogVisible.value = true
}

// 提交表单
const handleSubmit = async () => {
  if (!formRef.value) return

  await formRef.value.validate(async (valid) => {
    if (!valid) return

    try {
      submitting.value = true

      if (dialogMode.value === 'create') {
        await createUser({
          username: userForm.value.username,
          password: userForm.value.password,
          email: userForm.value.email || undefined,
          role: userForm.value.role,
        })
        ElMessage.success('创建用户成功')
      } else {
        await updateUser(userForm.value.id, {
          email: userForm.value.email,
          role: userForm.value.role,
          status: userForm.value.status as any,
        })
        ElMessage.success('更新用户成功')
      }

      dialogVisible.value = false
      await loadUsers()
    } catch (error) {
      ElMessage.error(dialogMode.value === 'create' ? '创建用户失败' : '更新用户失败')
      console.error(error)
    } finally {
      submitting.value = false
    }
  })
}

// 修改密码
const handleChangePassword = (user: User) => {
  passwordForm.value = {
    user_id: user.id,
    new_password: '',
    confirm_password: '',
  }
  passwordDialogVisible.value = true
}

// 提交密码修改
const handleSubmitPassword = async () => {
  if (!passwordFormRef.value) return

  await passwordFormRef.value.validate(async (valid) => {
    if (!valid) return

    try {
      submitting.value = true
      await changeUserPassword({
        user_id: passwordForm.value.user_id,
        new_password: passwordForm.value.new_password,
      })
      ElMessage.success('修改密码成功')
      passwordDialogVisible.value = false
    } catch (error) {
      ElMessage.error('修改密码失败')
      console.error(error)
    } finally {
      submitting.value = false
    }
  })
}

// 获取状态标签类型
const getStatusTagType = (status: string) => {
  const types: Record<string, any> = {
    Active: 'success',
    Suspended: 'warning',
    Deleted: 'danger',
  }
  return types[status] || 'info'
}

// 获取状态标签文本
const getStatusLabel = (status: string) => {
  const labels: Record<string, string> = {
    Active: '活跃',
    Suspended: '暂停',
    Deleted: '已删除',
  }
  return labels[status] || status
}

// 切换用户状态
const handleToggleStatus = async (user: User) => {
  const newStatus = user.status === 'Active' ? 'Suspended' : 'Active'
  const action = newStatus === 'Active' ? '激活' : '暂停'

  try {
    await ElMessageBox.confirm(`确定要${action}用户"${user.username}"吗？`, '确认操作', {
      confirmButtonText: '确定',
      cancelButtonText: '取消',
      type: 'warning',
    })

    await changeUserStatus(user.id, newStatus)
    ElMessage.success(`${action}用户成功`)
    await loadUsers()
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error(`${action}用户失败`)
      console.error(error)
    }
  }
}

// 删除用户
const handleDelete = async (user: User) => {
  try {
    await ElMessageBox.confirm(`确定要删除用户"${user.username}"吗？此操作不可恢复。`, '确认删除', {
      confirmButtonText: '确定',
      cancelButtonText: '取消',
      type: 'warning',
    })

    await deleteUser(user.id)
    ElMessage.success('删除用户成功')
    await loadUsers()
  } catch (error) {
    if (error !== 'cancel') {
      ElMessage.error('删除用户失败')
      console.error(error)
    }
  }
}

// 获取角色标签类型
const getRoleTagType = (role: string) => {
  const types: Record<string, any> = {
    Admin: 'danger',
    User: 'primary',
    ReadOnly: 'info',
  }
  return types[role] || 'info'
}

// 获取角色标签文本
const getRoleLabel = (role: string) => {
  const labels: Record<string, string> = {
    Admin: '管理员',
    User: '普通用户',
    ReadOnly: '只读用户',
  }
  return labels[role] || role
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
  loadUsers()
})
</script>

<style scoped lang="scss">
.users-container {
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
}
</style>
