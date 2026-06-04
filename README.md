# chatPM

本地优先的桌面 AI 聊天应用，基于 DeepSeek 大语言模型，所有聊天记录存储于本地 SQLite 数据库。

## 功能特性

- **多会话管理** — 创建、切换多个独立会话，会话列表按时间排序
- **流式对话** — 实时流式输出 AI 回复，支持思考模式（DeepSeek reasoning）
- **本地存储** — 全部聊天记录持久化于本地 SQLite，数据完全由用户掌控
- **上下文记忆** — 自动维护短期对话上下文，支持长对话摘要（规划中）
- **API 密钥配置** — 通过界面设置 DeepSeek API 密钥，即配即用
- **跨平台桌面应用** — 基于 Tauri 构建，支持 Windows、macOS、Linux

## 技术栈

| 层级 | 技术 |
|------|------|
| 桌面框架 | [Tauri 2.x](https://v2.tauri.app) |
| 前端框架 | [Svelte 5](https://svelte.dev) + [SvelteKit](https://kit.svelte.dev)（SPA 模式） |
| 前端语言 | TypeScript |
| 后端语言 | Rust（Edition 2024） |
| AI 接口 | DeepSeek API（流式 SSE） |
| 本地数据库 | SQLite（WAL 模式，通过 `rusqlite` bundled 编译） |
| 构建工具 | Vite |

## 开发环境要求

- [Rust](https://www.rust-lang.org)（stable 工具链）
- [Node.js](https://nodejs.org) 24+
- [Bun](https://bun.sh)（或 npm/pnpm）
- 系统依赖（Linux）：`libwebkit2gtk-4.1-dev`、`libgtk-3-dev`、`libappindicator3-dev` 等

## 快速开始

```bash
# 安装前端依赖
bun install

# 设置 DeepSeek API 密钥（也可在应用内设置）
export DEEPSEEK_API_KEY=your_api_key

# 启动开发模式
bun run tauri dev

# 构建生产版本
bun run tauri build
```

## 项目状态

**已实现：**
- 完整的对话交互界面（多会话、流式输出）
- 本地 SQLite 持久化存储
- DeepSeek API 流式集成
- API 密钥界面化配置
- 对话上下文记忆
- **P2P 设备间实时同步** — 三层消息协议（TurnBroadcast + StateBroadcast + P2P 补传）、类型状态机驱动的协议引擎

**规划中：**
- 长对话自动摘要压缩
- 端到端加密同步
- 多模型支持
- 对话导入/导出
- 自定义系统提示词

## P2P 同步架构

设备间通过 iroh gossip 网络实现实时会话同步。

### 协议消息

| 消息 | 触发条件 | 内容 |
|------|---------|------|
| `TurnBroadcast` | 本地新轮次（实时） | 轮次完整内容 |
| `StateBroadcast(Full)` | 新上线 / 邻居上线 | 全量会话水位 |
| `StateBroadcast(Incremental)` | 定期超时（30s） | 变更会话水位 |
| P2P `SyncRequest` | 收到水位广播后比对缺失 | 按需补传 |

### 核心组件

- **`SyncMachine<S>`**（`chat_pm_sync`）— 纯类型状态机，`handle(now, event) → OutEvent`，零 I/O
- **`SyncEngine`**（`chat_pm_service`）— I/O 容器，`poll_timeout()` 驱动事件循环
- **设备身份** — `DeviceId` = ed25519 公钥，从持久化私钥派生

### 技术栈

| 库 | 用途 |
|----|------|
| `iroh` 0.98 | P2P 端点、直连传输 |
| `iroh-gossip` 0.98 | Gossip 主题广播 |
| `distributed-topic-tracker` 0.3 | 分布式 topic 发现 |
| `ed25519-dalek` | 设备身份密钥 |

### 数据流

```
send_message → DB 写入 → handle_new_turn() → TurnBroadcast(gossip)
                                                    ↓
                                        其他节点 → WriteTurn(DB) + 乱序检测
```

## 许可证

MIT
