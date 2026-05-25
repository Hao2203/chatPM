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
| `chat_pm_commands`    | 业务逻辑管道、会话编排                  | 是（tokio）      | `PipelineError` |
| `src-tauri`（chatpm） | Tauri 应用二进制、Tauri 命令、应用状态  | 是               | `AppError`      |

### 依赖层次

```
chat_pm_session          ← 零内部依赖（仅 derive_more、uuid）
    ↑
chat_pm_database         ← + rusqlite（bundled）、chrono、serde
chat_pm_deepseek         ← + reqwest、secrecy、tokio
    ↑
chat_pm_commands         ← 依赖以上三个，+ uuid、tracing
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

## `chat_pm_commands` — 业务逻辑

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
| 命令层     | `PipelineError`       | `chat_pm_commands::session`                   | 组合 Chat + Api + Db + Internal              |
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

**`PipelineError`（`crates/chat_pm_commands/src/session.rs`）— 命令层统一错误：**

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

集成测试在 `crates/chat_pm_commands/src/tests.rs` 中 — 集成测试（`demo`）：

1. 从 `.env` 加载 `DEEPSEEK_API_KEY`
2. 创建 `ChatDb::open_in_memory()` + `ChatPipeline`
3. 运行多轮对话
4. 模拟跨"HTTP 请求"的会话恢复

运行：`cargo test --package chat_pm_commands`

### 前端

所有前端操作使用 **bun**（项目根目录执行）：

- 类型检查：`bun run check`（运行 `svelte-kit sync && svelte-check`）
- 构建：`bun run build`（运行 `vite build`）

---

## `chat_pm_sync` — 跨设备同步（P2P）

### 概述

`chat_pm_sync` 实现多设备间会话数据的 P2P 同步。核心思路是 **发布-订阅-拉取** 模型：节点通过 gossip 网络发布自身状态摘要，其他节点收到后比对差异，再通过直连 P2P 请求具体数据。

### iroh 技术栈层次

```
iroh (P2P 直连传输层)
  ├── iroh-gossip (主题广播 + 成员发现)
  │     └── distributed-topic-tracker (跨节点 topic 注册表)
  └── iroh-docs (基于 gossip 的 CRDT-like 状态同步)
        ├── DocTicket (文档读写权限凭证)
        └── SyncEngine 用其发布/订阅 SyncAnnouncement
```

| 库                          | 版本 | 用途                                                   |
| --------------------------- | ---- | ------------------------------------------------------ |
| `iroh`                      | 0.98 | 端点管理、P2P 直连、数据传输                           |
| `iroh-gossip`               | 0.98 | gossip topic 网络（节点发现、消息广播）                |
| `iroh-docs`                 | 0.98 | 基于 gossip 的 CRDT-like 文档同步（发布/订阅状态变更） |
| `distributed-topic-tracker` | 0.3  | 分布式 topic 发现（节点通过 topic hash 互相发现）      |

### DocTicket — 同步链的准入凭证（Newtype）

`DocTicket` 定义为领域 newtype，封装 iroh-docs 返回的 ticket 字符串：

```rust
// crates/chat_pm_sync/src/doc_ticket.rs
pub struct DocTicket(String);

impl DocTicket {
    pub fn from_string(s: String) -> Self;
    pub fn as_str(&self) -> &str;
    pub fn into_string(self) -> String;
}
impl fmt::Display for DocTicket { /* 直接输出内部字符串 */ }
impl Serialize / Deserialize for DocTicket { /* 序列化为字符串 */ }
```

`DocTicket` 是同步链的标识符：**拥有相同 ticket 的设备，同步相同的记录**。第一台设备创建文档获得 ticket，后续设备凭 ticket 加入同一同步链。

**设备加入同步链的两种角色：**

| 角色                   | 操作                                            | 说明                                      |
| ---------------------- | ----------------------------------------------- | ----------------------------------------- |
| **发起者（Author）**   | `SyncEngine::create_doc()` → 返回 `DocTicket`   | 第一台设备创建同步文档，获得 ticket       |
| **加入者（Follower）** | `SyncEngine::join_doc(ticket)` → 加入现有同步链 | 其他设备通过 ticket 加入，无需共享 secret |

**ticket 共享方式：**

- **文本粘贴**：发起者将 ticket 字符串发送给其他用户，加入者在 UI 中粘贴
- **二维码**：发起者将 ticket 渲染为二维码后，加入者扫码识别

**类型安全优势：**

- 不是裸 `String`，防止混入其他字符串参数
- 序列化时始终通过 `Display` 转为可分享的格式
- 调用方无法凭空构造——只能通过 `create_doc()` 获取

与此前基于共享 secret 的方案不同，ticket 机制的优势在于：

- **无需预共享密钥**：发起者创建 ticket，其他设备通过 ticket 接入
- **权限明确**：DocTicket 仅授予读权限，如需写权限需另外授权

### 同步流程

```
┌──────────────────────────────────────────────────────────────────┐
│  节点 A（本地变更）                                                 │
│                                                                  │
│  1. DB 变更（新增轮次/会话/标题）                                     │
│       ↓                                                          │
│  2. 构建 SessionWatermark 列表                                    │
│       ↓                                                          │
│  3. 发布 SyncAnnouncement 到 iroh-docs                            │
│       │                                                          │
│       ↓  (gossip 网络自动传播到其他节点)                              │
│                                                                  │
│  节点 B（接收端）                                                   │
│                                                                  │
│  4. iroh-docs 订阅回调收到 A 的 SyncAnnouncement                  │
│       ↓                                                          │
│  5. compute_sync_request(本地水位, 远程公告) → SyncRequest         │
│       ↓                                                          │
│  6. 通过 iroh 直连向节点 A 发送 SyncRequest                        │
│       ↓                                                          │
│  7. 节点 A 查询本地 DB 组装 SyncPayload 并返回                      │
│       ↓                                                          │
│  8. parse_sync_payload() → VerifiedPayload                       │
│       ↓                                                          │
│  9. 写入本地 DB（会话 + 轮次）                                      │
│       ↓                                                          │
│ 10. [可选] 节点 B 自身也变为"脏"状态，发布自己的 Announcement        │
└──────────────────────────────────────────────────────────────────┘
```

### 已实现的模块（纯领域层）

#### `device.rs` — 设备标识

- `DeviceId([u8; 32])` — 256-bit 设备唯一标识符
  - `generate()` — 生成随机 ID（两个 UUID v4 结合）
  - `from_hex(hex: &str)` — 从 64 hex 字符解析
  - `from_bytes(bytes: [u8; 32])`, `to_hex()`, `as_bytes()`
  - 序列化为 hex 字符串（JSON 友好）
- `DeviceIdError` — `InvalidFormat`（hex 校验失败）

#### `reconcile.rs` — 协调算法（纯函数，无 I/O）

**核心类型：**

| 类型               | 说明                                                                              |
| ------------------ | --------------------------------------------------------------------------------- |
| `SessionWatermark` | 设备对某会话的知识水位（`session_id`, `turn_count`, `has_title`, `created_at`）   |
| `SyncAnnouncement` | 本设备在 gossip 网络中的同步声明（`device_id` + `Vec<SessionWatermark>`）         |
| `SyncRequest`      | 同步请求（`need_sessions: Vec<SessionId>` + `need_turns: Vec<(SessionId, u64)>`） |
| `SessionSnapshot`  | 会话数据快照                                                                      |
| `TurnSnapshot`     | 轮次数据快照（含 `device_id` 来源标记）                                           |
| `SyncPayload`      | 同步响应负载（`Vec<SessionSnapshot>` + `Vec<TurnSnapshot>`）                      |
| `VerifiedPayload`  | 经过结构一致性验证的负载（保证无孤儿轮次、无重复 ID）                             |

**纯函数 API：**

| 函数                   | 签名                                                                     | 说明                               |
| ---------------------- | ------------------------------------------------------------------------ | ---------------------------------- |
| `compute_sync_request` | `(local: &[SessionWatermark], remote: &SyncAnnouncement) -> SyncRequest` | 比对双方水位，计算需要请求哪些数据 |
| `parse_sync_payload`   | `(payload: SyncPayload) -> Result<VerifiedPayload, SyncError>`           | 解析负载并验证结构一致性           |

**`compute_sync_request` 比对规则：**

| 本地有？ | 对方有？ | 轮次数比较     | 行为                              |
| -------- | -------- | -------------- | --------------------------------- |
| 否       | 是       | —              | 全量请求该会话（`need_sessions`） |
| 是       | 是       | 对方更多       | 请求缺失轮次（`need_turns`）      |
| 是       | 是       | 相同或本地更多 | 无需操作                          |
| 是       | 否       | —              | 无需操作（本地领先）              |

**`SyncError`（纯领域错误）：**

```rust
pub enum SyncError {
    OrphanedTurn(SessionId),    // 轮次的 session_id 不在会话列表中
    DuplicateSession(SessionId), // 重复的会话 ID
    DuplicateTurn(TurnId),       // 重复的轮次 ID
}
```

### 待实现：`sync_engine.rs` — 同步引擎（类型状态机）

> **文件位置：** `crates/chat_pm_commands/src/sync_engine.rs`（新建）

`SyncEngine<S>` 是同步功能的核心编排器，通过类型状态机驱动完整生命周期。所有底层 iroh 细节封装在内部，公共 API 不暴露任何 `iroh::` 类型。

**类型状态机流程图：**

```
SyncEngine<Disconnected>            // 未连接网络
    │
    └── init(db, config)
        │
        └── SyncEngine<Connected>          // 已加入 gossip 网络，未加入同步链
            │
            ├── create_doc(self)
            │   │
            │   └── (SyncEngine<Authoring>, DocTicket)  // 已创建 doc，持有 ticket
            │       │
            │       └── start(self)
            │           │
            │           └── SyncEngine<Syncing>        // 正在同步中
            │
            └── join_doc(self, ticket: DocTicket)
                │
                └── SyncEngine<Joined>                 // 已加入 doc
                    │
                    └── start(self)
                        │
                        └── SyncEngine<Syncing>        // 正在同步中

SyncEngine<Syncing>
    ├── publish_announcement(&self) → ()
    └── stop(self) → SyncEngine<Connected>            // 回到已连接状态
```

**结构体与状态定义：**

```rust
// 状态标记类型（零大小）
pub struct Disconnected;
pub struct Connected;
pub struct Authoring;
pub struct Joined;
pub struct Syncing;

/// 同步引擎，S 为当前生命周期状态
pub struct SyncEngine<S> {
    // 内部字段——全部为私有，不暴露底层 iroh 类型
    inner: SyncInner,
    db: Arc<Mutex<ChatDb>>,
    device_id: DeviceId,
    _state: PhantomData<S>,
}

// 内部类型，封装所有 iroh 依赖
struct SyncInner {
    endpoint: iroh::Endpoint,
    gossip: iroh::gossip::Gossip,
    docs: Option<iroh::docs::DocsEngine>,
    doc_id: Option<iroh::docs::DocId>,
    topic_tracker: distributed_topic_tracker::Client,
}
```

**各状态的方法 `impl` 块：**

```rust
impl SyncEngine<Disconnected> {
    /// 初始化网络连接：创建端点、加入 gossip 网络、连接 topic_tracker
    pub async fn init(
        db: Arc<Mutex<ChatDb>>,
        config: SyncConfig,
        device_id: DeviceId,
    ) -> Result<SyncEngine<Connected>, SyncError>;
}

impl SyncEngine<Connected> {
    /// 发起者：创建同步文档，获得 DocTicket
    /// 消耗 self，返回新状态 + ticket
    pub async fn create_doc(self) -> Result<(SyncEngine<Authoring>, DocTicket), SyncError>;

    /// 加入者：凭 ticket 加入已有同步链
    pub async fn join_doc(self, ticket: DocTicket) -> Result<SyncEngine<Joined>, SyncError>;
}

impl SyncEngine<Authoring> {
    /// 发起者启动同步
    pub async fn start(self) -> Result<SyncEngine<Syncing>, SyncError>;
}

impl SyncEngine<Joined> {
    /// 加入者启动同步
    pub async fn start(self) -> Result<SyncEngine<Syncing>, SyncError>;
}

impl SyncEngine<Syncing> {
    /// 本地发生变更后，发布状态广播
    pub async fn publish_announcement(&self) -> Result<(), SyncError>;

    /// 停止同步，回到已连接状态
    pub async fn stop(self) -> Result<SyncEngine<Connected>, SyncError>;
}
```

**类型安全保障：**

- `Disconnected` 状态下无法执行任何网络操作
- `Connected` 状态下可以选择 `create_doc` 或 `join_doc`，但不能发布公告
- `Authoring` / `Joined` 状态分别守卫发起者/加入者的启动路径
- `Syncing` 状态下可以发布公告，但不能再次创建/加入文档
- `create_doc(self)` 消耗 `Connected`，返回 `Authoring`——防止重复创建
- `join_doc(self)` 同样消耗 `Connected`——每个连接只能加入一个文档

**同步引擎内部流程：**

1. **初始化时（`init`）：**
   - 创建网络端点，绑定本地端口
   - 通过 topic_tracker 注册同步 topic
   - 加入 gossip 网络
   - 返回 `SyncEngine<Connected>`

2. **建立同步链：**
   - **发起者**调用 `create_doc(self)`：创建文档 → 获得 `DocTicket` → 持久化 ticket 到 DB
   - **加入者**调用 `join_doc(self, ticket)`：加入已有文档 → 持久化 ticket
   - 拥有相同 ticket 即属于同一同步链

3. **发布状态（`publish_announcement`）：**
   - 从 DB 查询所有会话的水位信息（构建 `Vec<SessionWatermark>`）
   - 组装 `SyncAnnouncement { device_id, sessions }`
   - 写入同步文档（底层通过 iroh-docs 的 CRDT merge 语义传播）

4. **订阅远端变化（后台循环，`start` 后自动运行）：**
   - 监听同步文档的变更事件
   - 解析为 `SyncAnnouncement`
   - 调用 `compute_sync_request(local, remote)` 计算差异
   - 若需要数据 → 向对端发送 `SyncRequest`（通过内部网络层）
   - 接收 `SyncPayload` → `parse_sync_payload` → `VerifiedPayload`
   - 写入本地 DB

### 待实现：网络传输层（模块内部，不公开）

> 网络传输逻辑封装在 `SyncEngine` 内部，不作为独立模块公开。

内部职责：

```rust
// 向对端发送同步请求并等待响应（内部函数）
async fn request_sync(
    endpoint: &iroh::Endpoint,
    peer: iroh::NodeId,
    request: SyncRequest,
) -> Result<SyncPayload, SyncError>;

// 处理接收到的同步请求（内部函数）
async fn handle_sync_request(
    db: Arc<Mutex<ChatDb>>,
    request: SyncRequest,
) -> Result<SyncPayload, SyncError>;
```

**传输协议（内部实现细节）：**

- 使用底层传输层建立连接
- 请求端：序列化 `SyncRequest` → 发送 → 等待响应 → 反序列化 `SyncPayload`
- 响应端：解析 `SyncRequest` → 查询 DB → 组装 `SyncPayload` → 返回

### 数据库层新增方法（`chat_pm_database`）

需在 `ChatDb` 中新增以下方法以支持同步：

| 方法                     | 签名                                                                      | 说明                                        |
| ------------------------ | ------------------------------------------------------------------------- | ------------------------------------------- |
| `build_watermarks`       | `(device_id: DeviceId) -> DbResult<Vec<SessionWatermark>>`                | 构建所有会话的本地水位列表                  |
| `get_session_snapshot`   | `(session_id: SessionId) -> DbResult<SessionSnapshot>`                    | 获取单个会话快照                            |
| `get_turns_from`         | `(session_id: SessionId, start_turn: u64) -> DbResult<Vec<TurnSnapshot>>` | 获取指定起始位置后的所有轮次                |
| `apply_verified_payload` | `(&self, payload: &VerifiedPayload) -> DbResult<usize>`                   | 将同步数据写入本地 DB（返回写入轮次数）     |
| `upsert_session`         | `(&self, snapshot: &SessionSnapshot) -> DbResult<()>`                     | 插入或更新会话记录                          |
| `upsert_turn`            | `(&self, snapshot: &TurnSnapshot) -> DbResult<()>`                        | 插入或更新轮次记录（基于 `turn_uuid` 去重） |

### Tauri 集成（`src-tauri`）

#### AppState 变化

```rust
struct AppState {
    db: std::sync::Mutex<ChatDb>,
    pipeline: Mutex<Option<ChatPipeline>>,
    sync_engine: Mutex<Option<SyncEngine>>,  // 新增：Option<SyncEngine<Syncing>>
}
```

#### 新增 Tauri 命令

| 命令              | 输入             | 输出                  | 说明                                                   |
| ----------------- | ---------------- | --------------------- | ------------------------------------------------------ |
| `create_sync_doc` | —                | `String`（DocTicket） | 发起者：创建同步 doc，返回 ticket 字符串供其他设备加入 |
| `join_sync_doc`   | `ticket: String` | `()`                  | 加入者：凭 ticket 加入已有同步链                       |
| `stop_sync`       | —                | `()`                  | 停止同步引擎                                           |
| `get_sync_status` | —                | `SyncStatus`          | 返回当前同步状态（在线节点数、最后同步时间等）         |

#### 事件

| 事件                  | payload                                         | 说明                           |
| --------------------- | ----------------------------------------------- | ------------------------------ |
| `sync-status-changed` | `{ status: string, peers: number }`             | 同步状态变化通知               |
| `sync-data-received`  | `{ session_count: number, turn_count: number }` | 收到新的同步数据后通知前端刷新 |

### 实现阶段

#### 第一阶段：网络基础设施（当前）

**目标：** 打通 iroh 网络连接，实现基本的 gossip + 状态发布订阅

| 文件                                         | 变更类型 | 内容                                                                  |
| -------------------------------------------- | -------- | --------------------------------------------------------------------- |
| `crates/chat_pm_commands/src/endpoint.rs`    | **实现** | `IrohEndpoint` 结构体 + `init()` / `join_topic()`                     |
| `crates/chat_pm_commands/src/sync_engine.rs` | **新建** | `SyncEngine` 结构体 + `init()` / `start()` / `publish_announcement()` |
| `crates/chat_pm_commands/Cargo.toml`         | 调整     | 确认 iroh/iroh-gossip/iroh-docs/distributed-topic-tracker 依赖        |

**验证：** 两台设备能通过 gossip 网络互相发现，收到对方的 `SyncAnnouncement`。

#### 第二阶段：数据查询与组装（数据库层）

**目标：** ChatDb 能够为同步提供水位信息，并按请求组装数据

| 文件                                 | 变更类型     | 内容                                                                 |
| ------------------------------------ | ------------ | -------------------------------------------------------------------- |
| `crates/chat_pm_database/src/lib.rs` | **新增方法** | `build_watermarks()` / `get_session_snapshot()` / `get_turns_from()` |
| `crates/chat_pm_sync/Cargo.toml`     | 调整         | 将 iroh/iroh-docs 从 dev-dependencies 移到正式 dependencies          |

**验证：** 单元测试覆盖各方法的正确性。

#### 第三阶段：P2P 数据传输

**目标：** 节点之间能通过 iroh 直连完成 SyncRequest → SyncPayload 的完整请求/响应

| 文件                                       | 变更类型 | 内容                                       |
| ------------------------------------------ | -------- | ------------------------------------------ |
| `crates/chat_pm_commands/src/transport.rs` | **新建** | `request_sync()` / `handle_sync_request()` |
| `crates/chat_pm_sync/src/reconcile.rs`     | 补充     | 可能需要序列化辅助方法                     |

**验证：** 两个节点间能完成实际数据传输和解析。

#### 第四阶段：数据写入与冲突处理

**目标：** 接收方将已验证的同步数据安全写入本地数据库

| 文件                                 | 变更类型     | 内容                                                              |
| ------------------------------------ | ------------ | ----------------------------------------------------------------- |
| `crates/chat_pm_database/src/lib.rs` | **新增方法** | `upsert_session()` / `upsert_turn()` / `apply_verified_payload()` |

**验证：** 测试重复数据不产生错误，增量同步正确合并。

#### 第五阶段：Tauri 集成 + 前端

**目标：** 用户通过 UI 控制同步功能，实时看到同步状态

| 文件                     | 变更类型 | 内容                                       |
| ------------------------ | -------- | ------------------------------------------ |
| `src-tauri/src/lib.rs`   | **修改** | AppState 增加 sync_engine、新增 Tauri 命令 |
| `src-tauri/src/error.rs` | **修改** | AppError 增加 SyncError 转换               |
| 前端 `+page.svelte`      | **修改** | 同步状态指示器、同步配置入口               |

**验证：** 端到端测试 — 设备 A 创建会话，设备 B 自动同步显示。

### 配置类型

```rust
pub struct SyncConfig {
    /// iroh 端点绑定的端口
    pub bind_port: Option<u16>,
    /// topic_tracker 服务地址
    pub topic_tracker_url: Option<String>,
    /// 设备名称（可选，用于 UI 区分）
    pub device_name: Option<String>,
}
```

### 安全考虑

- `DocTicket` 作为同步链准入凭证，拿到 ticket 即可同步对应数据，ticket 需安全传递
- 本地 `DeviceId` 持久化存储（`config` 表），重启后保持不变
- `TurnSnapshot` 携带 `device_id`，用于追踪数据来源和未来冲突解决
- 后续阶段将增加端到端加密层（用户数据加密后存储，同步传输密文）

---

## 当前状态

**已实现：**

- 核心领域模型（`chat_pm_session`）— 仅同步、无 I/O、newtype 模式
- 通过 `TitlePrompt` + 类型驱动状态机实现 AI 标题生成
- 通过 `rusqlite`（`chat_pm_database`）实现 SQLite 存储 — WAL 模式、bundled
- DeepSeek 流式客户端（`chat_pm_deepseek`）
- 带会话生命周期的聊天管道（`chat_pm_commands`）
- 基于事件流式传输的 Tauri 命令（`src-tauri`）
- 聊天 UI，含会话列表、标题显示、流式传输、API key 配置（SvelteKit）
- **同步基础类型与协调算法**（`chat_pm_sync`）— `DeviceId`、`DocTicket`、`SessionWatermark`、`SyncAnnouncement`、`SyncRequest`/`SyncPayload`、`VerifiedPayload`、`compute_sync_request()`、`parse_sync_payload()`
- **数据库同步基础设施**（`chat_pm_database`）— `device_id` 列、`devices` 表、`turn_uuid` 全局唯一标识

**尚未实现：**

- **同步网络层**（`endpoint.rs` 为空，`sync_engine.rs` 未建）— iroh/gossip/docs 初始化、topic 发现、状态发布订阅
- **P2P 数据传输**（`transport.rs` 未建）— 基于 iroh 直连的请求/响应
- **数据库同步查询/写入方法** — `build_watermarks()`、`upsert_session()`、`upsert_turn()`、`apply_verified_payload()`
- **Tauri 同步命令与事件** — `create_sync_doc`、`join_sync_doc`、`stop_sync`、`get_sync_status`、同步状态事件
- **前端同步 UI** — 状态指示器、配置入口
