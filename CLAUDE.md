# chatPM Project Skill

## Overview

chatPM is a local-first chat application with future end-to-end encrypted sync support. All chat records are stored locally in SQLite. The tech stack is **Rust workspace** (core logic + Tauri backend) + **Tauri 2.x** (desktop shell) + **SvelteKit 5** (UI, SPA mode).

---

## Architecture

### Workspace Crates (Rust)

| Crate | Purpose | Async? |
|---|---|---|
| `chat_pm_session` | Core domain types & pure functions | **No** (sync-only) |
| `chat_pm_database` | SQLite storage via `rusqlite` (`bundled`) | No |
| `chat_pm_deepseek` | DeepSeek API streaming client | Yes (tokio) |
| `chat_pm_commands` | Business logic pipeline, session orchestration | Yes (tokio) |
| `src-tauri` (chatpm) | Tauri app binary, Tauri commands, app state | Yes |

### Dependency Hierarchy

```
chat_pm_session          ← zero internal deps (only derive_more, uuid)
    ↑
chat_pm_database         ← + rusqlite (bundled), chrono, serde
chat_pm_deepseek         ← + reqwest, secrecy, tokio
    ↑
chat_pm_commands         ← depends on all three above, + uuid, tracing
    ↑
src-tauri (chatpm)       ← depends on all crates, + tauri, tokio, uuid
```

---

## `chat_pm_session` — Core Domain (Sync Only)

**Rule:** This crate MUST NOT contain any async functions, tokio, or I/O. It defines pure data types and transformations.

### Key Types

| File | Types | Notes |
|---|---|---|
| `chat.rs` | `TurnId(u64)`, `Role`(System/User/Assistant), `StopReason`, `MessageFrame`, `ReplyReceiver`, `FinalAnswer`, `MemoryUpdatePlan` | Streaming reply assembly |
| `message.rs` | `UserInput`, `ChatMessage` | `UserInput::new()` normalizes whitespace |
| `memory.rs` | `Memory { user_text, assistant_text }` | One turn pair |
| `context.rs` | `Context { summary: Option<Summary>, recent_memory: Vec<Memory> }` | Assembled before prompt composition |
| `summary.rs` | `Summary { content, last_turn_id }` | Conversation summary for long contexts |
| `language.rs` | `Language` enum (~30 variants), `SUPPORTED_LANGUAGES` | Each variant has a `code()` → BCP-47 string |
| `prompt.rs` | `SystemPrompt`, `PromptComposer`, `TitlePrompt` | `TitlePrompt` carries `SessionId` + user input; `compose()` → `Vec<ChatMessage>` |
| `session.rs` | `SessionId(Uuid)`, `Title(String)`, `NewSession`, `Session` | Newtype wrappers + lifecycle states |

### Prompt Composition Flow (`PromptComposer::compose_prompt`)

1. If no recent memory → prepend `SystemPrompt` as first message
2. If summary exists → prepend `"Summary: {content}"` as system message
3. Interleave memory pairs: assistant msg → user msg (oldest first)
4. Append current `UserInput` as final user message

### Title Generation Flow (Type-Driven State Machine)

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

### Domain Newtype Pattern

核心层对所有外部标识符使用 newtype 封装，杜绝裸 `String` / `Uuid`：

| Newtype | 内部类型 | 关键 trait |
|---|---|---|
| `SessionId` | `Uuid` | `Copy`, `Display`, `Hash` |
| `Title` | `String` | `Display`, `as_str()`, `into_inner()` |
| `UserInput` | `String` | `Display`, `Into<String>` |

---

## `chat_pm_database` — SQLite Storage

### Implementation

Uses `rusqlite` with `bundled` feature (SQLite compiled into binary). Thread-safe via `Arc<Mutex<Connection>>`.

### Schema

```sql
CREATE TABLE sessions (
    session_id  TEXT PRIMARY KEY,
    created_at  TEXT NOT NULL,   -- RFC 3339
    title       TEXT,            -- AI-generated or user-set
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

Configured with `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`.

### Key Methods

| Method | Description |
|---|---|
| `MemoryDb::open(path)` | Open/create persistent SQLite file |
| `MemoryDb::open_in_memory()` | Open in-memory DB (tests) |
| `create_session(session_id)` | Insert new session |
| `session_exists(session_id) -> bool` | Check existence |
| `get_session(session_id) -> Option<SessionRecord>` | Fetch full record (includes title) |
| `set_session_title(session_id, title)` | Update session title |
| `get_session_title(session_id) -> Option<String>` | Read title |
| `list_sessions() -> Vec<SessionRecord>` | All sessions, newest first |
| `append_chat_turn(session_id, user_text, assistant_text)` | Insert one turn pair |
| `recent_turns(session_id, n) -> Vec<TurnRecord>` | Last N turns (chronological) |
| `load_recent_memory(session_id, n) -> Vec<Memory>` | Last N turns as Memory pairs |
| `next_turn_id(session_id) -> TurnId` | `MAX(turn_num) + 1` |
| `stats() -> DbStats` | Session & turn counts |

### Key Types

- `SessionRecord { session_id, created_at, title, user_persona }` — Serialize/Deserialize
- `TurnRecord { turn_id, session_id, user_text, assistant_text, created_at }` — `to_memory_chunk() -> Memory`

### Utility

`cosine_similarity(a: &[f32], b: &[f32]) -> f32` — pure Rust, for future vector/RAG search.

---

## `chat_pm_deepseek` — API Client

### Key Types

- `ApiKey(SecretString)` — validates chars, wraps in `secrecy::SecretString`
- `Client { http, api_base, api_key }` — defaults to `https://api.deepseek.com`
- `ChatRequestConfig { model, max_tokens, thinking_enabled, reasoning_effort }`
- `ChatChunk { raw_text, completion_tokens, stop_reason }`
- `ReasoningEffort` — `High` | `Max`

### Streaming Flow

`Client::stream_chat()` → POST to `/chat/completions` with `stream: true` → parse SSE `data:` lines → `mpsc::Receiver<Result<ChatChunk>>`

Stop reasons: `"length"` → `MaxTokens`, `"content_filter"` → `ContentFilter`, other → `EndOfSequence`

---

## `chat_pm_commands` — Business Logic

### `ChatPipeline`

Orchestrates the full flow with type-driven session lifecycle:

| Method | Signature | Notes |
|---|---|---|
| `create_session` | `() → NewSession` | DB record created, no title yet |
| `finalize_session` | `(TitlePrompt) → Result<Session>` | Calls LLM, persists title, **consumes** `TitlePrompt` |
| `resume_session` | `(SessionId) → Result<Session>` | Only succeeds if title exists in DB |
| `chat` | `(&Session, UserInput) → Result<Receiver<Result<MessageFrame>>>` | Type system ensures only titled sessions can chat |

**State machine flow (first turn):**
```
create_session() → NewSession
    → new_session.into_title_prompt(user_input) → TitlePrompt
    → pipeline.finalize_session(tp) → Session
    → pipeline.chat(&session, user_input)
```

**Subsequent turns:** `resume_session(id) → Session` → `chat(&session, input)`

### `PipelineConfig` (Default)

| Field | Default |
|---|---|
| `chat_model` | `"deepseek-v4-flash"` |
| `token_limit` | 8192 |
| `reply_token_limit` | 2048 |
| `short_term_turns` | 6 |
| `long_term_top_k` | 4 |
| `system_role` | Chinese assistant prompt |
| `thinking_enabled` | false |
| `reasoning_effort` | None |

Env override: `CHAT_PM_REASONING_EFFORT`

---

## Tauri Integration (`src-tauri`)

### AppState

```rust
struct AppState {
    db: MemoryDb,                          // persistent SQLite
    pipeline: Mutex<Option<ChatPipeline>>,  // initialized after API key set
}
```

Database is stored in Tauri's app data directory (`$DATA_DIR/chatpm.db`).

### Configuration Persistence

API key is stored in the `config` table of the SQLite database (`key="api_key"`). On startup, `setup()` attempts to load and validate the stored key, auto-initializing the pipeline if valid.

### Tauri Commands

| Command | Input | Output | Notes |
|---|---|---|---|
| `check_api_key` | — | `bool` | Whether pipeline is ready |
| `create_session` | — | `String` (session_id) | Creates `NewSession` in DB, returns UUID |
| `set_api_key` | `api_key: String` | `()` | Validates, stores to DB, inits pipeline |
| `send_message` | `session_id, content` | `()` | State machine: `NewSession`→`TitlePrompt`→`Session` on first turn |
| `list_sessions` | — | `Vec<SessionInfo>` | All sessions, newest first |
| `get_turns` | `session_id` | `Vec<TurnInfo>` | All turns for session |
| `update_session_title` | `session_id, title` | `()` | Manual title edit, emits event |

### Event-Based Streaming

`send_message` spawns a tokio task that emits:
- `chat-chunk` → `{ session_id, content }` — each text chunk
- `chat-done` → `{ session_id }` — stream finished, turn stored in DB
- `session-title-updated` → `{ session_id, title }` — emitted on first-turn title generation and manual title edits

This avoids blocking the Tauri IPC channel during streaming.

---

## Frontend (Tauri + SvelteKit)

- **SvelteKit 5** with runes (`$state`, `$effect`, etc.)
- **SPA mode**: `adapter-static` + `ssr = false` (no Node.js server)
- **Tauri 2.x** with `@tauri-apps/api` v2
- Frontend calls Rust via `invoke("command_name", { args })`
- Listens to streaming events via `listen("chat-chunk", callback)`

### UI Structure (`+page.svelte`)

```
┌──────────┬──────────────────────────────┐
│ Sidebar  │  Chat Area                   │
│          │                              │
│ Sessions │  Messages (user/assistant)   │
│ + New    │                              │
│ Settings │                              │
│          ├──────────────────────────────┤
│          │  Input Box  [Send]           │
└──────────┴──────────────────────────────┘
```

- **Settings panel**: modal overlay for entering DeepSeek API key
- **Streaming**: cursor blink animation on in-progress messages
- **Session list**: sidebar with created_at timestamps, active highlight

---

## Key Conventions

### Code Style
- Edition 2024 for all crates except `src-tauri` (edition 2021)
- Workspace lints: `clippy::dbg_macro = "warn"`
- Error handling: `anyhow` for application, `thiserror` (workspace dep, not yet used)
- Logging: `tracing` with `logforth` bridge (configured in tests)
- Date/time stored as RFC 3339 strings in SQLite

### Data Flow
```
[UI] invoke("send_message", {session_id, content})
         ↓
[Tauri Command] → pipeline.chat() → tokio::spawn
         ↓                              ↓
[mpsc stream] ← DeepSeek SSE       emit("chat-chunk")
         ↓                              ↓
[ReplyReceiver] → FinalAnswer      [UI] listen() → update messages
         ↓
[DB] append_chat_turn()
         ↓
emit("chat-done")
```

### Security
- API keys MUST use `chat_pm_deepseek::ApiKey` (wraps `secrecy::SecretString`)
- API key entered via UI settings, stored in-memory only (no disk persistence)
- Never log or serialize raw API keys
- Future: end-to-end encryption for sync (not yet implemented)

---

## Testing

Test in `crates/chat_pm_commands/src/tests.rs` — integration test (`demo`):
1. Loads `.env` for `DEEPSEEK_API_KEY`
2. Creates `MemoryDb::open_in_memory()` + `ChatPipeline`
3. Runs multi-turn conversation
4. Simulates session resume across "HTTP requests"

Run: `cargo test --package chat_pm_commands`

---

## Current State

**Implemented:**
- Core domain model (`chat_pm_session`) — sync-only, no I/O, newtype pattern
- AI-powered title generation via `TitlePrompt` + type-driven state machine
- SQLite storage via `rusqlite` (`chat_pm_database`) — WAL mode, bundled
- DeepSeek streaming client (`chat_pm_deepseek`)
- Chat pipeline with session lifecycle (`chat_pm_commands`)
- Tauri commands with event-based streaming (`src-tauri`)
- Chat UI with session list, title display, streaming, API key config (SvelteKit)

**Not Yet Implemented:**
- Summary/compression for long conversations (placeholder types exist)
- Vector embedding / RAG integration (cosine_similarity ready)
- End-to-end encrypted sync
- Multi-model support (currently DeepSeek only)
- Conversation export / import
- User persona / custom system prompt UI
