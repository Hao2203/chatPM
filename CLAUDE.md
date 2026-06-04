# chatPM 项目技能

## 语言要求

**Agent 必须使用中文回复用户。** 代码注释和提交信息建议用中文。

## 概述

chatPM 是一个本地优先的聊天应用，未来将支持端到端加密同步。所有聊天记录本地存储在 SQLite 中。技术栈为 **Rust workspace**（核心逻辑 + Tauri 后端）+ **Tauri 2.x**（桌面壳）+ **SvelteKit 5**（UI，SPA 模式）。前端使用 **bun** 作为包管理器和运行时。

---

## 架构

### Workspace Crates（Rust）

| Crate                 | 用途                                    | 异步？           | 错误类型        |
| --------------------- | --------------------------------------- | ---------------- | --------------- |
| `chat_pm_session`     | 核心领域类型和纯函数                    | **否**（仅同步） | `ChatError`     |
| `chat_pm_database`    | 通过 `rusqlite`（`bundled`）存储 SQLite | 否               | `DbError`       |
| `chat_pm_deepseek`    | DeepSeek API 流式客户端                 | 是（tokio）      | `ApiError`      |
| `chat_pm_service`     | 业务逻辑管道、会话编排                  | 是（tokio）      | `PipelineError` |
| `src-tauri`（chatpm） | Tauri 应用二进制、Tauri 命令、应用状态  | 是               | `AppError`      |

### 依赖层次

```
chat_pm_session          ← 零内部依赖（仅 derive_more、uuid）
    ↑
chat_pm_database         ← + rusqlite（bundled）、chrono、serde
chat_pm_deepseek         ← + reqwest、secrecy、tokio
    ↑
chat_pm_service          ← 依赖以上三个，+ uuid、tracing
    ↑
src-tauri（chatpm）       ← 依赖所有 crate，+ tauri、tokio、uuid
```

---

## `chat_pm_session` — 核心领域（仅同步）

**规则：** 此 crate 不得包含任何 async 函数、tokio 或 I/O。它定义纯数据类型和转换。

### 关键类型

| 文件          | 类型                                                                                                                           | 说明                                                                        |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `chat.rs`     | `TurnId(u64)`、`Role`(System/User/Assistant)、`StopReason`、`MessageFrame`、`ReplyReceiver`、`FinalAnswer`、`MemoryUpdatePlan` | 流式回复组装                                                                |
| `message.rs`  | `UserInput`、`ChatMessage`                                                                                                     | `UserInput::new()` 规范化空白字符                                           |
| `memory.rs`   | `Memory { user_text, assistant_text }`                                                                                         | 一对轮次                                                                    |
| `context.rs`  | `Context { summary: Option<Summary>, recent_memory: Vec<Memory> }`                                                             | 在提示词组装前构建                                                          |
| `summary.rs`  | `Summary { content, last_turn_id }`                                                                                            | 长对话的对话摘要                                                            |
| `language.rs` | `Language` 枚举（约 30 个变体）、`SUPPORTED_LANGUAGES`                                                                         | 每个变体有 `code()` → BCP-47 字符串                                         |
| `prompt.rs`   | `SystemPrompt`、`PromptComposer`、`TitlePrompt`                                                                                | `TitlePrompt` 携带 `SessionId` + 用户输入；`compose()` → `Vec<ChatMessage>` |
| `session.rs`  | `SessionId(Uuid)`、`Title(String)`、`NewSession`、`Session`                                                                    | Newtype 封装 + 生命周期状态                                                 |

### 提示词组装流程（`PromptComposer::compose_prompt`）

1. 如果没有近期记忆 → 将 `SystemPrompt` 作为第一条消息
2. 如果存在摘要 → 将 `"Summary: {content}"` 作为系统消息插入
3. 交替插入记忆对：助手消息 → 用户消息（最旧的在前）
4. 将当前 `UserInput` 追加为最后一条用户消息

### 标题生成流程（类型驱动的状态机）

```
NewSession { session_id: SessionId }
    │
    └── .into_title_prompt(user_input)         // 消耗 NewSession
        │
        └── TitlePrompt { session_id, user_input }
            │
            └── .compose() → Vec<ChatMessage>   // 纯函数：组装提示词
            │
            └── pipeline.finalize_session(tp)   // 调用 LLM，持久化标题
                │
                └── Session { session_id, title: Title }
                    │
                    └── pipeline.chat(&session, user_input)
```

**类型安全保障：**

- `NewSession` 不能直接对话 — 没有 `chat` 方法
- `into_title_prompt(self)` 消耗 `NewSession`，防止重复生成标题
- `finalize_session(TitlePrompt)` 消耗 `TitlePrompt`，标题生成仅一次
- 恢复已有标题的会话：`pipeline.resume_session(SessionId) → Result<Session>`

### 领域 Newtype 模式

核心层对所有外部标识符使用 newtype 封装，杜绝裸 `String` / `Uuid`：

| Newtype     | 内部类型 | 关键 trait                            |
| ----------- | -------- | ------------------------------------- |
| `SessionId` | `Uuid`   | `Copy`、`Display`、`Hash`             |
| `Title`     | `String` | `Display`、`as_str()`、`into_inner()` |
| `UserInput` | `String` | `Display`、`Into<String>`             |

---

## `chat_pm_database` — SQLite 存储

### 实现

使用 `rusqlite` 配合 `bundled` 特性（SQLite 编译进二进制文件）。通过 `Arc<Mutex<Connection>>` 实现线程安全。

### 数据库模式

```sql
CREATE TABLE sessions (
    session_id  TEXT PRIMARY KEY,
    created_at  TEXT NOT NULL,   -- RFC 3339
    title       TEXT,            -- AI 生成或用户设置
    user_persona TEXT
);

CREATE TABLE turns (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    TEXT NOT NULL,
    turn_num      INTEGER NOT NULL,
    user_text     TEXT NOT NULL,
    assistant_text TEXT NOT NULL,
    created_at    TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id),
    UNIQUE(session_id, turn_num)
);

CREATE TABLE config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
```

配置了 `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`。

### 关键方法

| 方法                                                      | 说明                        |
| --------------------------------------------------------- | --------------------------- |
| `ChatDb::open(path)`                                      | 打开/创建持久化 SQLite 文件 |
| `ChatDb::open_in_memory()`                                | 打开内存数据库（测试用）    |
| `create_session(session_id)`                              | 插入新会话                  |
| `session_exists(session_id) -> bool`                      | 检查是否存在                |
| `get_session(session_id) -> Option<SessionRecord>`        | 获取完整记录（含标题）      |
| `set_session_title(session_id, title)`                    | 更新会话标题                |
| `get_session_title(session_id) -> Option<String>`         | 读取标题                    |
| `list_sessions() -> Vec<SessionRecord>`                   | 所有会话，最新的在前        |
| `append_chat_turn(session_id, user_text, assistant_text)` | 插入一对轮次                |
| `recent_turns(session_id, n) -> Vec<TurnRecord>`          | 最近 N 轮（按时间顺序）     |
| `load_recent_memory(session_id, n) -> Vec<Memory>`        | 最近 N 轮作为 Memory 对     |
| `next_turn_id(session_id) -> TurnId`                      | `MAX(turn_num) + 1`         |
| `stats() -> DbStats`                                      | 会话和轮次计数              |

### 关键类型

- `SessionRecord { session_id, created_at, title, user_persona }` — 可序列化/反序列化
- `TurnRecord { turn_id, session_id, user_text, assistant_text, created_at }` — `to_memory_chunk() -> Memory`

### 工具函数

`cosine_similarity(a: &[f32], b: &[f32]) -> f32` — 纯 Rust 实现，用于未来的向量/RAG 搜索。

---

## `chat_pm_deepseek` — API 客户端

### 关键类型

- `ApiKey(SecretString)` — 验证字符，封装在 `secrecy::SecretString` 中
- `Client { http, api_base, api_key }` — 默认使用 `https://api.deepseek.com`
- `ChatRequestConfig { model, max_tokens, thinking_enabled, reasoning_effort }`
- `ChatChunk { raw_text, completion_tokens, stop_reason }`
- `ReasoningEffort` — `High` | `Max`

### 流式处理流程

`Client::stream_chat()` → POST 到 `/chat/completions`，`stream: true` → 解析 SSE `data:` 行 → `mpsc::Receiver<Result<ChatChunk>>`

停止原因：`"length"` → `MaxTokens`，`"content_filter"` → `ContentFilter`，其他 → `EndOfSequence`

---

## `chat_pm_service` — 业务逻辑

### `ChatPipeline`

通过类型驱动的会话生命周期编排完整流程：

| 方法               | 签名                                                             | 说明                                         |
| ------------------ | ---------------------------------------------------------------- | -------------------------------------------- |
| `create_session`   | `() → NewSession`                                                | 创建 DB 记录，暂无标题                       |
| `finalize_session` | `(TitlePrompt) → Result<Session>`                                | 调用 LLM，持久化标题，**消耗** `TitlePrompt` |
| `resume_session`   | `(SessionId) → Result<Session>`                                  | 仅在 DB 中存在标题时成功                     |
| `chat`             | `(&Session, UserInput) → Result<Receiver<Result<MessageFrame>>>` | 类型系统确保只有有标题的会话才能对话         |

**状态机流程（首轮）：**

```
create_session() → NewSession
    → new_session.into_title_prompt(user_input) → TitlePrompt
    → pipeline.finalize_session(tp) → Session
    → pipeline.chat(&session, user_input)
```

**后续轮次：** `resume_session(id) → Session` → `chat(&session, input)`

### `PipelineConfig`（默认值）

| 字段                | 默认值                |
| ------------------- | --------------------- |
| `chat_model`        | `"deepseek-v4-flash"` |
| `token_limit`       | 8192                  |
| `reply_token_limit` | 2048                  |
| `short_term_turns`  | 6                     |
| `long_term_top_k`   | 4                     |
| `system_role`       | 中文助手提示词        |
| `thinking_enabled`  | false                 |
| `reasoning_effort`  | None                  |

环境变量覆盖：`CHAT_PM_REASONING_EFFORT`

---

## Tauri 集成（`src-tauri`）

### AppState

```rust
struct AppState {
    db: ChatDb,                          // 持久化 SQLite
    pipeline: Mutex<Option<ChatPipeline>>,  // 设置 API key 后初始化
}
```

数据库存储在 Tauri 的应用数据目录中（`$DATA_DIR/chatpm.db`）。

### 配置持久化

API key 存储在 SQLite 数据库的 `config` 表中（`key="api_key"`）。启动时，`setup()` 尝试加载并验证已存储的 key，如果有效则自动初始化 pipeline。

### Tauri 命令

| 命令                   | 输入                  | 输出                   | 说明                                              |
| ---------------------- | --------------------- | ---------------------- | ------------------------------------------------- |
| `check_api_key`        | —                     | `bool`                 | pipeline 是否就绪                                 |
| `create_session`       | —                     | `String`（session_id） | 在 DB 中创建 `NewSession`，返回 UUID              |
| `set_api_key`          | `api_key: String`     | `()`                   | 验证、存储到 DB、初始化 pipeline                  |
| `send_message`         | `session_id, content` | `()`                   | 状态机：首轮 `NewSession`→`TitlePrompt`→`Session` |
| `list_sessions`        | —                     | `Vec<SessionInfo>`     | 所有会话，最新的在前                              |
| `get_turns`            | `session_id`          | `Vec<TurnInfo>`        | 会话的所有轮次                                    |
| `update_session_title` | `session_id, title`   | `()`                   | 手动编辑标题，发出事件                            |

### 基于事件的流式传输

`send_message` 创建一个 tokio 任务，发出：

- `chat-chunk` → `{ session_id, content }` — 每个文本块
- `chat-done` → `{ session_id }` — 流结束，轮次已存入 DB
- `session-title-updated` → `{ session_id, title }` — 首轮标题生成和手动标题编辑时发出

这避免了在流式传输期间阻塞 Tauri IPC 通道。

---

## 前端（Tauri + SvelteKit）

- **SvelteKit 5** 配合 runes（`$state`、`$effect` 等）
- **SPA 模式**：`adapter-static` + `ssr = false`（无 Node.js 服务器）
- **Tauri 2.x** 配合 `@tauri-apps/api` v2
- 前端通过 `invoke("command_name", { args })` 调用 Rust
- 通过 `listen("chat-chunk", callback)` 监听流式事件

### UI 结构（`+page.svelte`）

```
┌──────────┬──────────────────────────────┐
│ 侧边栏    │  聊天区域                      │
│          │                              │
│ 会话列表  │  消息（用户/助手）              │
│ + 新建   │                              │
│ 设置     │                              │
│          ├──────────────────────────────┤
│          │  输入框  [发送]               │
└──────────┴──────────────────────────────┘
```

- **设置面板**：用于输入 DeepSeek API key 的模态覆盖层
- **流式传输**：进行中消息的光标闪烁动画
- **会话列表**：侧边栏显示 created_at 时间戳，高亮当前活跃会话

---

## 关键约定

### 代码风格

- 所有 crate 使用 Edition 2024，`src-tauri` 除外（edition 2021）
- Workspace lints：`clippy::dbg_macro = "warn"`
- 日志：`tracing` 配合 `logforth` 桥接（在测试中配置）
- 日期/时间以 RFC 3339 字符串形式存储在 SQLite 中
- **前端组件属性（Props）必须使用驼峰命名法**，例如 `onApiKeyChange` 而非 `onapikeychange`，`onUpdateTitle` 而非 `onupdatetitle`

### 错误处理

**禁止使用 `unwrap()` 和 `expect()`。** 所有可能失败的操作必须通过 `Result` 传播或显式处理。

**四层错误体系：**

| 层级       | 错误类型              | 位置                                          | 用途                                         |
| ---------- | --------------------- | --------------------------------------------- | -------------------------------------------- |
| 领域层     | `ChatError`           | `chat_pm_session::error`                      | 违反业务逻辑约束（会话不存在、标题未生成等） |
| 外部接口层 | `ApiError`、`DbError` | `chat_pm_deepseek::error`、`chat_pm_database` | API 调用失败、数据库操作失败                 |
| 命令层     | `PipelineError`       | `chat_pm_service::session`                    | 组合 Chat + Api + Db + Internal              |
| 接口层     | `AppError`            | `src-tauri::error`                            | Tauri 命令返回值，序列化为 `{kind, message}` |

**`ChatError`（`crates/chat_pm_session/src/error.rs`）— 纯粹的业务逻辑违反：**

```rust
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChatError {
    #[error("会话 '{0}' 不存在")]         SessionNotFound(String),
    #[error("会话 '{0}' 尚未生成标题")]     TitleNotGenerated(String),
    #[error("未配置 API Key")]           ApiKeyNotConfigured,
    #[error("无效的 API Key")]           InvalidApiKey,
}
```

- 不包含任何 I/O 或基础设施故障，只反映领域规则被打破
- `Clone` 实现允许在调用方匹配具体变体后重试

**`ApiError`（`crates/chat_pm_deepseek/src/error.rs`）— 外部 API 调用失败：**

```rust
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("API 请求发送失败: {0}")]     RequestFailed(String),
    #[error("API 返回错误状态: {0}")]     ErrorStatus(String),
    #[error("API 响应解析失败: {0}")]     ParseFailed(String),
    #[error("模型未返回任何 choice")]     NoChoice,
}
```

- `stream_chat()` 返回 `Result<Receiver<Result<ChatChunk, ApiError>>, ApiError>`
- `chat_complete()` 返回 `Result<String, ApiError>`
- `from_env()` 返回 `Result<Self, ChatError>`（API key 验证是领域规则）

**`DbError`（`crates/chat_pm_database/src/lib.rs`）— 数据库操作失败：**

```rust
#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("数据库锁已污染")]    Lock,
    #[error("SQL 错误: {0}")]    Sql(#[from] rusqlite::Error),
    #[error("日期解析失败: {0}")]  DateParse(String),
}

pub type DbResult<T> = Result<T, DbError>;
```

- `ChatDb` 所有公共方法返回 `DbResult<T>`
- `lock_conn()` 私有方法封装 `Mutex::lock()`，返回 `DbResult<MutexGuard<_>>`

**`PipelineError`（`crates/chat_pm_service/src/session.rs`）— 命令层统一错误：**

```rust
#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("[Chat Error] {0}")] Chat(#[from] ChatError),
    #[error("[Database Error] {0}")] Db(#[from] DbError),
    #[error("[API Error] {0}")] Api(#[from] ApiError),
    #[error("[Internal Error] {0}")] Internal(#[from] anyhow::Error),
}
```

- `ChatPipeline` 所有方法返回 `Result<T, PipelineError>`
- `From` 自动转换子错误，调用方可匹配具体变体（如 `send_message` 中对 `SessionNotFound` | `TitleNotGenerated` 的特殊处理）

**`AppError`（`src-tauri/src/error.rs`）— Tauri 接口序列化：**

```rust
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    pub kind: String,     // "db" | "api" | "validation" | "locked" | "internal"
    pub message: String,
}
```

- 实现 `From<ChatError>`（kind=`"validation"`）、`From<DbError>`（kind=`"db"`）、`From<ApiError>`（kind=`"api"`）、`From<PipelineError>`（按变体分发）、`From<anyhow::Error>`（kind=`"internal"`）
- 所有 Tauri 命令返回 `Result<T, AppError>`，前端通过 `getErrorMessage(e)` 提取消息

**前端错误处理：**

```typescript
function getErrorMessage(e: any): string {
    if (typeof e === "string") return e;
    if (e?.message) return e.message;
    return String(e);
}
```

**错误 Display 格式：** 所有错误类型的 `Display` 实现必须说明错误**类别**和**说明**，不得简单转发底层错误。具体规则：

- **底层错误**（`ChatError`、`DbError`、`ApiError`）：每条 `#[error("...")]` 消息以自然语言描述错误，包含上下文信息（如错误的会话 ID、原因等）。内部已自带前缀（如 `ApiError` 的 `"API ..."`、`DbError::Sql` 的 `"SQL error: "`）。
- **组合错误**（`PipelineError`）：使用 `[Category] description` 格式，类别必须有实际意义，能够直观反映错误来源。通过 `#[error("[Category] {0}")]` 在转发时添加类型前缀。
- **接口错误**（`AppError`）：`Display` 输出 `[kind] message`，`kind` 字段作为类别标识。

示例 Display 输出：

```
[Chat Error] Session 'abc-123' not found
[Database Error] SQL error: table not found
[API Error] API returned error status: 401
[Internal Error] connection reset
[db] SQL error: table not found                  ← AppError 前端展示
[validation] Session 'abc-123' not found          ← AppError 前端展示
```

**错误流转链路（send_message 示例）：**

```
[DeepSeek API] ApiError ──┐
[SQLite]      DbError  ──┼── PipelineError ──→ AppError ──→ [Frontend] getErrorMessage(e)
[Chat]  ChatError ─────┘        ↑                    ↑
                            ? 自动转换          From 逐变体分发
```

### 数据流

```
[UI] invoke("send_message", {session_id, content})
         ↓
[Tauri Command] → pipeline.chat() → tokio::spawn
         ↓                              ↓
[mpsc stream] ← DeepSeek SSE       emit("chat-chunk")
         ↓                              ↓
[ReplyReceiver] → FinalAnswer      [UI] listen() → 更新消息
         ↓
[DB] append_chat_turn()
         ↓
emit("chat-done")
```

### 安全

- API key 必须使用 `chat_pm_deepseek::ApiKey`（封装 `secrecy::SecretString`）
- API key 通过 UI 设置输入，仅存储在内存中（不持久化到磁盘）
- 绝不记录或序列化原始 API key
- 未来：同步的端到端加密（尚未实现）

---

## 测试

### Rust 后端

集成测试在 `crates/chat_pm_service/src/tests.rs` 中 — 集成测试（`demo`）：

1. 从 `.env` 加载 `DEEPSEEK_API_KEY`
2. 创建 `ChatDb::open_in_memory()` + `ChatPipeline`
3. 运行多轮对话
4. 模拟跨"HTTP 请求"的会话恢复

运行：`cargo test --package chat_pm_service`

### 前端

所有前端操作使用 **bun**（项目根目录执行）：

- 类型检查：`bun run check`（运行 `svelte-kit sync && svelte-check`）
- 构建：`bun run build`（运行 `vite build`）

---

## `chat_pm_sync` — 跨设备同步（P2P）

### 概述

`chat_pm_sync` 实现多设备间会话数据的 P2P 实时同步。协议分三层消息体系：

| 消息 | 触发条件 | 内容 | 接收方行为 |
|------|---------|------|-----------|
| `TurnBroadcast` | 本地新轮次（实时） | 轮次完整内容 | 直接写入 DB |
| `StateBroadcast(Full)` | 新上线 / 邻居上线 | 全量会话水位 | 比对后 P2P 补传 |
| `StateBroadcast(Incremental)` | 定期超时（30s） | 变更会话水位 | 比对后 P2P 补传 |
| P2P `SyncRequest` | 收到水位广播后比对发现缺失 | 缺失数据请求 | 按需应答 |

### 设备标识

`DeviceId` 是 ed25519 公钥（32 字节），从 `device_secret_key`（持久化于 DB）的私钥派生。`DeviceId` 等同于 `iroh::EndpointId`，两者转换不可失败。

```rust
DeviceId::from_secret_key(&secret_key_bytes)  // 私钥 → 公钥 → DeviceId
DeviceId::generate_identity()                 // 生成随机设备身份 → (DeviceId, 私钥)
```

### 协议类型（`crates/chat_pm_sync/src/reconcile.rs`）

```rust
// Gossip 通道消息信封
#[serde(tag = "type")]
enum GossipMessage {
    TurnBroadcast(TurnBroadcast),   // 实时增量轮次
    StateBroadcast(StateBroadcast), // 水位广播
}

struct TurnBroadcast { device_id, session_id, turn_num, user_text, assistant_text, created_at }
struct StateBroadcast { device_id, kind: StateKind, sessions: Vec<SessionWatermark> }
enum StateKind { Full, Incremental }

// 状态机输入/输出
enum InEvent {
    NewLocalTurn(TurnSnapshot),
    RemoteTurn { from_device: DeviceId, turn: TurnSnapshot },
    RemoteState { from_device: DeviceId, sessions: Vec<SessionWatermark> },
    RemoteSession(SessionSnapshot),
    NeighborUp,
    Leave,
    Timeout,
}

enum OutEvent {
    BroadcastGossip(GossipMessage),
    WriteTurn(TurnSnapshot),
    WriteSession(SessionSnapshot),
    RequestBackfill { to_device: DeviceId, request: SyncRequest },
}
```

### 状态机（`crates/chat_pm_sync/src/sync_machine.rs`）

纯类型驱动的 `SyncMachine<S>`，零 I/O：

```
SyncDisconnected
  → into_syncing(ticket, watermarks) → SyncSyncing

SyncSyncing
  ├── handle(now, NeighborUp) → StateBroadcast(Full)
  ├── handle(now, NewLocalTurn) → TurnBroadcast (实时)
  ├── handle(now, RemoteTurn) → 乱序检测 + WriteTurn + RequestBackfill
  ├── handle(now, RemoteState) → compute_request() → RequestBackfill
  ├── handle(now, Timeout) → StateBroadcast(Incremental)（脏会话水位）
  └── into_disconnected → SyncDisconnected
```

**核心 API：**

```rust
impl SyncMachine<SyncSyncing> {
    pub fn handle(&mut self, now: Instant, event: InEvent) -> impl Iterator<Item = OutEvent> + '_;
    pub fn poll_timeout(&self) -> Option<Instant>;  // 下次超时绝对时间点
    pub fn watermarks(&self) -> Vec<SessionWatermark>;
    pub fn ticket(&self) -> &SyncTicket;
    pub fn into_disconnected(self) -> SyncMachine<SyncDisconnected>;
}
```

**乱序处理：** `SessionState` 内部用 `BTreeSet<turn_num>` 追踪已收轮次 + `contiguous` 连续前缀。收到间隙轮次（`turn_num > contiguous + 1`）时自动产出 `RequestBackfill` 填补缺口。

**增量广播脏标记：** `SessionState.dirty: bool` — 新轮次置 `true`，Timeout 广播后置 `false`。避免每次超时都广播全量水位。

### 同步引擎（`crates/chat_pm_service/src/sync_engine.rs`）

`SyncEngine` 是非泛型 I/O 容器，内部状态机在后台事件循环中运行：

```rust
pub struct SyncEngine {
    input_tx: mpsc::UnboundedSender<(Instant, InEvent)>,  // 外部事件注入
    ticket: SyncTicket,
    device_id: DeviceId,
    secret_key: [u8; 32],
    _bg: BackgroundSyncHandle,
}

impl SyncEngine {
    pub async fn create(db, config, secret_key: Option<[u8; 32]>) -> Result<Self>;
    pub async fn join(db, config, secret_key: Option<[u8; 32]>, ticket) -> Result<Self>;
    pub fn handle_new_turn(&self, now: Instant, turn: TurnSnapshot);  // send_message 后调用
    pub fn handle_neighbor_up(&self, now: Instant);                   // 手动触发全量广播
    pub fn ticket(&self) -> &SyncTicket;
    pub fn device_id(&self) -> DeviceId;
    pub fn secret_key_bytes(&self) -> [u8; 32];
}
```

**事件循环（后台 tokio 任务）：**

```text
loop {
    timeout = machine.poll_timeout();
    select! {
        gossip_rx.next()  → handle_gossip_event() → machine.handle() → dispatch(OutEvent)
        input_rx.recv()   → machine.handle(event)  → dispatch(OutEvent)
        sleep(timeout)    → machine.handle(Timeout) → dispatch(OutEvent)
    }
}
```

**`dispatch(OutEvent)` 分发：**

| OutEvent | 执行 |
|----------|------|
| `BroadcastGossip(msg)` | `topic.gossip_sender().broadcast(json)` |
| `WriteTurn(turn)` | `db.upsert_turn(&turn)` |
| `WriteSession(s)` | `db.upsert_session(record)` |
| `RequestBackfill { to_device, request }` | P2P 直连 `request_sync()`，结果通过 `backfill_tx` 回传状态机 |

### iroh 技术栈

```
iroh (P2P 直连传输层)
  └── iroh-gossip (主题广播 + 成员发现)
        └── distributed-topic-tracker (跨节点 topic 注册表)
```

| 库 | 版本 | 用途 |
|----|------|------|
| `iroh` | 0.98 | 端点管理、P2P 直连、数据传输 |
| `iroh-gossip` | 0.98 | gossip topic 网络（节点发现、消息广播） |
| `distributed-topic-tracker` | 0.3 | 分布式 topic 发现 + DHT 引导 |

### SyncTicket — 同步链凭证

`SyncTicket` 封装 `distributed_topic_tracker::TopicId`（256-bit hash）。通过 base64url 编码为可分享的字符串。

### 数据库同步方法（`chat_pm_database`）

| 方法 | 签名 | 说明 |
|------|------|------|
| `build_watermarks` | `(device_id) -> Vec<SessionWatermark>` | 所有会话水位 |
| `get_session_snapshot` | `(session_id) -> Option<SessionSnapshot>` | 单会话快照 |
| `get_turns_from` | `(session_id, start_turn) -> Vec<TurnSnapshot>` | 指定起始轮次 |
| `upsert_turn` | `(&TurnSnapshot) -> ()` | 基于 `turn_uuid` 去重 |
| `upsert_session` | `(SessionRecord) -> ()` | 插入或更新会话 |
| `apply_verified_payload` | `(&VerifiedPayload) -> usize` | 写入已验证负载 |

### Tauri 集成

#### AppState

```rust
struct AppState {
    db: Arc<Mutex<ChatDb>>,
    db_path: PathBuf,
    service: Mutex<Option<ChatService>>,
    sync_engine: Mutex<Option<SyncEngine>>,
    device_id: DeviceId,
}
```

设备身份在启动时通过 `load_or_create_identity()` 初始化：从 `config` 表读取 `device_secret_key`，派生 `DeviceId`；首次启动时生成随机 ed25519 密钥并持久化。

#### 同步 Tauri 命令

| 命令 | 输入 | 输出 | 说明 |
|------|------|------|------|
| `get_sync_status` | — | `SyncStatusPayload` | 查询同步状态 |
| `init_and_create_sync_doc` | — | `String`（ticket） | 发起者：创建同步链 |
| `join_sync_doc` | `ticket: String` | `()` | 加入者：凭 ticket 加入 |
| `stop_sync` | — | `()` | 停止同步 |
| `publish_sync_announcement` | — | `()` | 手动触发全量广播 |

#### `send_message` 与同步的集成

`send_message` 完成后自动调用 `engine.handle_new_turn()`，将新轮次注入状态机触发 `TurnBroadcast` 实时广播：

```rust
// send_message 的 tokio::spawn 任务末尾：
let app_state = app_handle.state::<AppState>();
let turn = db.recent_turns(sid, 1)?.first().cloned()?;
let snapshot = TurnSnapshot { ... };
let engine_guard = app_state.sync_engine.lock().await;
if let Some(ref engine) = *engine_guard {
    engine.handle_new_turn(Instant::now(), snapshot);
}
```

#### 启动时自动恢复

`restore_sync_engine()` 在 `setup()` 中异步调用，从 DB 读取 `sync_state`、`sync_secret_key`、`sync_ticket`，若上次关闭时同步活跃则自动调用 `SyncEngine::join()` 恢复。

#### 事件

| 事件 | payload | 说明 |
|------|---------|------|
| `sync-status-changed` | `{ status, active, ticket }` | 同步状态变化 |

### 数据流

```
[UI] invoke("send_message")
         ↓
[Tauri Command] → chat_service.chat() → stream
         ↓                                    ↓
[tokio::spawn] emit("chat-chunk")       DeepSeek SSE
         ↓
[chat-done] emit("chat-done")
         ↓
[SyncEngine] handle_new_turn() → channel
         ↓
[后台循环] machine.handle(NewLocalTurn)
         ↓
dispatch(BroadcastGossip(TurnBroadcast))
         ↓
[gossip] → 其他节点收到 → RemoteTurn → WriteTurn + 乱序检测
```

### 安全

- `DeviceId` = ed25519 公钥，私钥 `device_secret_key` 持久化于 DB 的 `config` 表
- `SyncTicket` 作为同步链准入凭证，需安全传递
- `TurnSnapshot` 携带 `device_id` 追踪数据来源
- 后续将增加端到端加密层

---

## 当前状态

**已实现：**

- 核心领域模型（`chat_pm_session`）— 仅同步、无 I/O、newtype 模式
- 通过 `TitlePrompt` + 类型驱动状态机实现 AI 标题生成
- 通过 `rusqlite`（`chat_pm_database`）实现 SQLite 存储 — WAL 模式、bundled
- DeepSeek 流式客户端（`chat_pm_deepseek`）
- 带会话生命周期的聊天管道（`chat_pm_service`）
- 基于事件流式传输的 Tauri 命令（`src-tauri`）
- 聊天 UI，含会话列表、标题显示、流式传输、API key 配置（SvelteKit）
- **同步协议状态机**（`chat_pm_sync`）— `SyncMachine<S>` 类型状态机、`InEvent`/`OutEvent` 事件驱动、`GossipMessage` 三层消息体系、乱序检测（`BTreeSet` + `contiguous`）
- **同步引擎**（`chat_pm_service`）— `SyncEngine` I/O 容器、`poll_timeout()` 驱动事件循环、`TurnBroadcast` 实时广播 + P2P 直连补传、`handle_new_turn()` / `handle_neighbor_up()` API
- **设备身份** — `DeviceId` = ed25519 公钥，从 `device_secret_key` 派生，与 `EndpointId` 不可失败互转
- **数据库同步方法** — `build_watermarks()`、`get_session_snapshot()`、`get_turns_from()`、`upsert_turn()`、`upsert_session()`、`apply_verified_payload()`
- **Tauri 同步命令** — `get_sync_status`、`init_and_create_sync_doc`、`join_sync_doc`、`stop_sync`、`publish_sync_announcement`
- **`send_message` 自动触发同步** — 轮次完成后自动 `handle_new_turn()`

**尚未实现：**

- 长对话自动摘要压缩
- 多模型支持
- 对话导入/导出
- 自定义系统提示词
