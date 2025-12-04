<template>
  <el-dialog
    v-model="visible"
    title="文件浏览器"
    width="70%"
    :close-on-click-modal="false"
    @close="handleClose"
  >
    <el-table
      v-loading="loading"
      :data="fileList"
      style="width: 100%"
      height="500"
    >
      <el-table-column prop="file_path" label="文件路径" min-width="200" />
      <el-table-column prop="file_size" label="文件大小" width="120">
        <template #default="{ row }">
          {{ formatBytes(row.file_size) }}
        </template>
      </el-table-column>
      <el-table-column prop="created_at" label="创建时间" width="180" />
      <el-table-column prop="modified_at" label="修改时间" width="180" />
      <el-table-column label="操作" width="180" fixed="right">
        <template #default="{ row }">
          <el-button
            type="primary"
            size="small"
            link
            @click="handleDownload(row)"
          >
            下载
          </el-button>
          <el-button
            type="danger"
            size="small"
            link
            @click="handleDelete(row)"
          >
            删除
          </el-button>
        </template>
      </el-table-column>
    </el-table>
  </el-dialog>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'
import { getFileList, downloadFile, deleteFile } from '@/api/files'
import type { FileItem } from '@/types/files'

interface Props {
  modelValue: boolean
}

interface Emits {
  (e: 'update:modelValue', value: boolean): void
  (e: 'refresh'): void
}

const props = defineProps<Props>()
const emit = defineEmits<Emits>()

const visible = ref(false)
const loading = ref(false)
const fileList = ref<FileItem[]>([])

// 监听 modelValue 变化
watch(
  () => props.modelValue,
  (newVal) => {
    visible.value = newVal
    if (newVal) {
      loadFileList()
    }
  },
  { immediate: true }
)

// 监听 visible 变化
watch(visible, (newVal) => {
  emit('update:modelValue', newVal)
})

// 格式化字节数
const formatBytes = (bytes: number): string => {
  if (bytes === 0) return '0 B'
  const k = 1024
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB']
  const i = Math.floor(Math.log(bytes) / Math.log(k))
  return Math.round((bytes / Math.pow(k, i)) * 100) / 100 + ' ' + sizes[i]
}

// 加载文件列表
const loadFileList = async () => {
  try {
    loading.value = true
    fileList.value = await getFileList()
  } catch (error) {
    console.error('Failed to load file list:', error)
    ElMessage.error('加载文件列表失败')
  } finally {
    loading.value = false
  }
}

// 下载文件
const handleDownload = (file: FileItem) => {
  const url = downloadFile(file.file_id)
  window.open(url, '_blank')
}

// 删除文件
const handleDelete = async (file: FileItem) => {
  try {
    await ElMessageBox.confirm(
      `确定要删除文件 "${file.file_path}" 吗？`,
      '提示',
      {
        confirmButtonText: '确定',
        cancelButtonText: '取消',
        type: 'warning',
      }
    )

    await deleteFile(file.file_id)
    ElMessage.success('删除成功')
    loadFileList()
    emit('refresh')
  } catch (error) {
    if (error !== 'cancel') {
      console.error('Failed to delete file:', error)
      ElMessage.error('删除失败')
    }
  }
}

// 关闭对话框
const handleClose = () => {
  visible.value = false
}
</script>

<style scoped lang="scss">
// 可以添加一些自定义样式
</style>
