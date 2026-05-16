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
chat_pm_session          ← zero internal deps (only derive_more)
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
| `prompt.rs` | `SystemPrompt`, `PromptComposer` | Composes `Vec<ChatMessage>` from context + user input |

### Prompt Composition Flow (`PromptComposer::compose_prompt`)

1. If no recent memory → prepend `SystemPrompt` as first message
2. If summary exists → prepend `"Summary: {content}"` as system message
3. Interleave memory pairs: assistant msg → user msg (oldest first)
4. Append current `UserInput` as final user message

---

## `chat_pm_database` — SQLite Storage

### Implementation

Uses `rusqlite` with `bundled` feature (SQLite compiled into binary). Thread-safe via `Arc<Mutex<Connection>>`.

### Schema

```sql
CREATE TABLE sessions (
    session_id  TEXT PRIMARY KEY,
    created_at  TEXT NOT NULL,   -- RFC 3339
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
```

Configured with `PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;`.

### Key Methods

| Method | Description |
|---|---|
| `MemoryDb::open(path)` | Open/create persistent SQLite file |
| `MemoryDb::open_in_memory()` | Open in-memory DB (tests) |
| `create_session(session_id)` | Insert new session |
| `session_exists(session_id) -> bool` | Check existence |
| `list_sessions() -> Vec<SessionRecord>` | All sessions, newest first |
| `append_chat_turn(session_id, user_text, assistant_text)` | Insert one turn pair |
| `recent_turns(session_id, n) -> Vec<TurnRecord>` | Last N turns (chronological) |
| `load_recent_memory(session_id, n) -> Vec<Memory>` | Last N turns as Memory pairs |
| `next_turn_id(session_id) -> TurnId` | `MAX(turn_num) + 1` |
| `stats() -> DbStats` | Session & turn counts |

### Key Types

- `SessionRecord { session_id, created_at, user_persona }` — Serialize/Deserialize
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

Orchestrates the full flow:
1. Load recent memory from DB
2. Build `SystemPrompt` from config
3. Compose prompt via `PromptComposer`
4. Stream from DeepSeek client
5. Collect response via `ReplyReceiver` → `FinalAnswer`
6. Write turn back to DB via `append_chat_turn`

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

### Session Management

- `SessionHandle(Uuid)` / `SessionId(Uuid)` — UUIDv7
- `SessionId::from_uuid(uuid)` — constructor for external use
- `create_session()` → new UUIDv7, inserted into DB
- `resume_session(SessionId)` → validates existence, returns handle
- `Display` on both types outputs the UUID string

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
| `create_session` | — | `String` (session_id) | UUIDv7 |
| `set_api_key` | `api_key: String` | `()` | Validates, stores to DB, inits pipeline |
| `send_message` | `session_id, content` | `()` | Emits events for streaming |
| `list_sessions` | — | `Vec<SessionInfo>` | All sessions, newest first |
| `get_turns` | `session_id` | `Vec<TurnInfo>` | All turns for session |

### Event-Based Streaming

`send_message` spawns a tokio task that emits:
- `chat-chunk` → `{ session_id, content }` — each text chunk
- `chat-done` → `{ session_id }` — stream finished, turn stored in DB

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
- Core domain model (`chat_pm_session`) — sync-only, no I/O
- SQLite storage via `rusqlite` (`chat_pm_database`) — WAL mode, bundled
- DeepSeek streaming client (`chat_pm_deepseek`)
- Chat pipeline with session management (`chat_pm_commands`)
- Tauri commands with event-based streaming (`src-tauri`)
- Chat UI with session list, streaming display, API key config (SvelteKit)

**Not Yet Implemented:**
- Summary/compression for long conversations (placeholder types exist)
- Vector embedding / RAG integration (cosine_similarity ready)
- End-to-end encrypted sync
- Multi-model support (currently DeepSeek only)
- Conversation export / import
- User persona / custom system prompt UI
