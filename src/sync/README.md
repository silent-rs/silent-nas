# 同步功能模块 (sync)

## 概述

`sync` 模块包含了 Silent-NAS 的所有同步相关功能，采用模块化设计，分为三个主要子模块：

- **CRDT 同步** (`crdt`) - 基于 CRDT 的元数据同步
- **增量同步** (`incremental`) - 基于块的文件差异检测和增量同步
- **节点同步** (`node`) - 跨节点的文件同步功能

## 目录结构

```
src/sync/
├── README.md                 # 本文档
├── mod.rs                    # 模块入口和公共导出
├── crdt.rs                   # CRDT 元数据同步（原sync.rs）
├── incremental/              # 增量同步子模块
│   ├── mod.rs               # 增量同步模块入口
│   ├── core.rs              # 核心算法（块签名、差异检测）
│   ├── handler.rs           # 同步处理器（协调增量同步流程）
│   ├── api.rs               # HTTP API 处理逻辑
│   └── incremental_sync_api_README.md  # API 文档
└── node/                     # 节点同步子模块
    ├── mod.rs               # 节点同步模块入口
    ├── manager.rs           # 节点管理和同步协调
    ├── client.rs            # gRPC 客户端
    └── service.rs           # gRPC 服务端
```

## 模块说明

### 1. CRDT 同步 (`crdt`)

**文件**: `src/sync/crdt.rs`

**功能**:
- 基于 CRDT (Conflict-free Replicated Data Types) 的元数据同步
- LWW-Register (Last-Write-Wins Register) 实现
- 向量时钟冲突检测
- 文件同步状态管理

**主要类型**:
- `SyncManager` - 同步管理器
- `FileSync` - 文件同步状态

**使用示例**:
```rust
use crate::sync::crdt::SyncManager;

let sync_manager = SyncManager::new("node-1".to_string());
```

### 2. 增量同步 (`incremental`)

**目录**: `src/sync/incremental/`

#### 2.1 核心算法 (`core.rs`)

**功能**:
- 文件块签名计算（SHA256 强哈希 + Adler-32 弱哈希）
- 块级差异检测
- 差异块提取和应用
- 传输节省统计

**主要类型**:
- `IncrementalSyncManager` - 增量同步管理器
- `FileSignature` - 文件签名
- `ChunkInfo` - 块信息
- `DeltaChunk` - 差异块
- `SyncDelta` - 同步差异信息

**使用示例**:
```rust
use crate::sync::incremental::IncrementalSyncManager;

let manager = IncrementalSyncManager::new(64 * 1024); // 64KB 块大小
let signature = manager.calculate_signature("file_id", &data)?;
```

#### 2.2 同步处理器 (`handler.rs`)

**功能**:
- 增量拉取文件（优先增量，失败时回退到全量）
- 与存储层和 HTTP 客户端集成
- 自动哈希验证
- 传输节省统计

**主要类型**:
- `IncrementalSyncHandler` - 增量同步处理器

**使用示例**:
```rust
use crate::sync::incremental::IncrementalSyncHandler;

let handler = IncrementalSyncHandler::new(storage, 64 * 1024);
let data = handler.pull_incremental(file_id, source_http_addr).await?;
```

#### 2.3 HTTP API (`api.rs`)

**功能**:
- 提供增量同步的 HTTP API 处理逻辑
- 从 `main.rs` 中分离业务逻辑
- 易于测试和维护

**主要函数**:
- `handle_get_signature` - 处理文件签名请求
- `handle_get_delta` - 处理差异块请求

**API 端点**:
- `GET /api/sync/signature/{id}` - 获取文件签名
- `POST /api/sync/delta/{id}` - 获取文件差异块

### 3. 节点同步 (`node`)

**目录**: `src/sync/node/`

#### 3.1 节点管理 (`manager.rs`)

**功能**:
- 节点信息管理
- 节点发现和注册
- 心跳机制
- 跨节点同步协调

**主要类型**:
- `NodeManager` - 节点管理器
- `NodeSyncCoordinator` - 同步协调器
- `NodeInfo` - 节点信息

#### 3.2 gRPC 客户端 (`client.rs`)

**功能**:
- gRPC 客户端实现
- 连接管理
- 节点通信

**主要类型**:
- `NodeSyncClient` - gRPC 客户端
- `ClientConfig` - 客户端配置

#### 3.3 gRPC 服务端 (`service.rs`)

**功能**:
- gRPC 服务端实现
- 处理节点间的同步请求
- Protobuf 类型转换

**主要类型**:
- `NodeSyncServiceImpl` - gRPC 服务实现

## 模块导入

### 在项目内部使用

```rust
// CRDT 同步
use crate::sync::crdt::{SyncManager, FileSync};

// 增量同步 - 核心
use crate::sync::incremental::{
    IncrementalSyncManager, FileSignature, DeltaChunk, SyncDelta
};

// 增量同步 - 处理器
use crate::sync::incremental::IncrementalSyncHandler;

// 增量同步 - API
use crate::sync::incremental::api::{handle_get_signature, handle_get_delta};

// 节点同步
use crate::sync::node::{NodeManager, NodeSyncCoordinator, NodeInfo};
use crate::sync::node::client::{NodeSyncClient, ClientConfig};
use crate::sync::node::service::NodeSyncServiceImpl;
```

### 向后兼容性

为了保持向后兼容，顶层 `sync` 模块仍然导出常用类型：

```rust
use crate::sync::{SyncManager, FileSync};  // 等同于 sync::crdt::*
```

## 设计原则

1. **模块化** - 每个功能模块独立，职责单一
2. **可测试性** - 业务逻辑与框架分离，便于单元测试
3. **可维护性** - 清晰的目录结构和模块划分
4. **向后兼容** - 保持现有代码的导入路径可用
5. **关注点分离** - 算法、处理、API 分离

## 测试

```bash
# 测试所有同步模块
cargo test --bin silent-nas sync::

# 测试 CRDT 同步
cargo test --bin silent-nas sync::crdt

# 测试增量同步
cargo test --bin silent-nas sync::incremental

# 测试节点同步
cargo test --bin silent-nas sync::node
```

## 迁移指南

### 从旧路径迁移

| 旧路径 | 新路径 |
|--------|--------|
| `use crate::sync::SyncManager;` | `use crate::sync::crdt::SyncManager;` |
| `use crate::incremental_sync::*;` | `use crate::sync::incremental::*;` |
| `use crate::incremental_sync_handler::*;` | `use crate::sync::incremental::IncrementalSyncHandler;` |
| `use crate::incremental_sync_api::*;` | `use crate::sync::incremental::api::*;` |
| `use crate::node_sync::*;` | `use crate::sync::node::*;` |
| `use crate::node_sync_client::*;` | `use crate::sync::node::client::*;` |
| `use crate::node_sync_service::*;` | `use crate::sync::node::service::*;` |

**注意**: 顶层的 `sync::SyncManager` 仍然可用，作为便捷导入。

## 未来扩展

- [ ] 添加更多差异算法（rsync算法优化）
- [ ] 支持多播同步
- [ ] P2P 节点发现
- [ ] 同步性能监控和指标
- [ ] 自适应块大小
- [ ] 压缩传输支持
