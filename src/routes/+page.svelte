<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import Sidebar from "$lib/components/Sidebar.svelte";
  import MessageList from "$lib/components/MessageList.svelte";
  import ChatInput from "$lib/components/ChatInput.svelte";
  import SettingsModal from "$lib/components/SettingsModal.svelte";
  import ConfirmDialog from "$lib/components/ConfirmDialog.svelte";
  import KbSelector from "$lib/components/KbSelector.svelte";
  import KbManageModal from "$lib/components/KbManageModal.svelte";

  // ── Types ──────────────────────────────────────────────────
  interface Message {
    id: number;
    role: "user" | "assistant";
    content: string;
    streaming: boolean;
  }

  interface SessionInfo {
    session_id: string;
    created_at: string;
    title: string | null;
  }

  // ── State ──────────────────────────────────────────────────
  let apiKey = $state("");
  let apiKeyConfigured = $state(false);
  let model = $state("deepseek-v4-flash");
  let showSettings = $state(false);
  let loading = $state(false);
  let sidebarCollapsed = $state(false);

  let sessions = $state<SessionInfo[]>([]);
  let activeSessionId = $state<string | null>(null);
  let pendingNewChat = $state(false);
  let messages = $state<Message[]>([]);
  let inputText = $state("");
  let messageId = $state(0);
  let sending = $state(false);
  let sessionToDelete = $state<string | null>(null);
  let contextTokens = $state(0);
  const CONTEXT_WINDOW = 1_048_576; // must match ChatConfig.context_window

  // ── Sync state ────────────────────────────────────────────
  let syncActive = $state(false);
  let syncLoading = $state(false);
  let syncTicket = $state("");

  function formatNumber(n: number): string {
    return n.toLocaleString("en-US");
  }

  // 提取错误消息（兼容 AppError { kind, message } 和字符串）
  function getErrorMessage(e: any): string {
    if (typeof e === "string") return e;
    if (e?.message) return e.message;
    return String(e);
  }

  // Per-chat draft storage
  let drafts = $state<Record<string, string>>({});

  // ── Knowledge Base state ─────────────────────────────────────
  interface KbInfo {
    kb_id: string;
    name: string;
    created_at: string;
    document_count: number;
    total_chunks: number;
  }
  let knowledgeBases = $state<KbInfo[]>([]);
  let activeKbIds = $state<string[]>([]);
  let showKbManager = $state(false);

  async function loadKnowledgeBases() {
    try {
      knowledgeBases = await invoke<KbInfo[]>("list_knowledge_bases");
    } catch (_) {
      // Knowledge service may not be initialized
    }
  }

  async function loadSessionKbRefs(sid: string) {
    try {
      activeKbIds = await invoke<string[]>("get_session_kb_refs", { sessionId: sid });
    } catch (_) {
      activeKbIds = [];
    }
  }

  async function toggleKbRef(kbId: string) {
    const newIds = activeKbIds.includes(kbId)
      ? activeKbIds.filter(id => id !== kbId)
      : [...activeKbIds, kbId];
    activeKbIds = newIds;
    if (activeSessionId) {
      try {
        await invoke("set_session_kb_refs", { sessionId: activeSessionId, kbIds: newIds });
      } catch (_) {}
    }
  }

  // ── Event cleanup ──────────────────────────────────────────
  let unlistenChunk: UnlistenFn | null = null;
  let unlistenDone: UnlistenFn | null = null;

  function draftKey(): string {
    return activeSessionId ?? "__new__";
  }

  function saveDraft() {
    const key = activeSessionId ?? (pendingNewChat ? "__new__" : null);
    if (key !== null) {
      drafts = { ...drafts, [key]: inputText };
    }
  }

  function loadDraft(forSessionId: string | null) {
    const key = forSessionId ?? "__new__";
    inputText = drafts[key] ?? "";
  }

  // ── Load sessions ──────────────────────────────────────────
  async function loadSessions() {
    sessions = await invoke<SessionInfo[]>("list_sessions");
  }

  // ── Delete session ──────────────────────────────────────────
  async function confirmDeleteSession() {
    const sid = sessionToDelete;
    if (!sid) return;
    sessionToDelete = null;
    try {
      await invoke("delete_session", { sessionId: sid });
      if (activeSessionId === sid) {
        activeSessionId = null;
        pendingNewChat = false;
        messages = [];
        messageId = 0;
        const { [sid]: _, ...rest } = drafts;
        drafts = rest;
      }
      await loadSessions();
    } catch (e: any) {
      alert("删除失败：" + getErrorMessage(e));
    }
  }

  // ── Update session title ───────────────────────────────────
  async function updateSessionTitle(sid: string, title: string) {
    await invoke("update_session_title", { sessionId: sid, title });
    // Optimistic local update
    const s = sessions.find((s) => s.session_id === sid);
    if (s) {
      s.title = title;
      sessions = [...sessions];
    }
  }

  // ── Start new chat (lazy: no backend call yet) ─────────────
  function startNewChat() {
    saveDraft();
    activeSessionId = null;
    pendingNewChat = true;
    messages = [];
    messageId = 0;
    contextTokens = 0;
    activeKbIds = [];
    loadDraft(null);
  }

  // ── Select existing session ────────────────────────────────
  async function selectSession(sid: string) {
    saveDraft();
    activeSessionId = sid;
    pendingNewChat = false;
    loadDraft(sid);
    loadSessionKbRefs(sid);

    const turns = await invoke<
      {
        turn_num: number;
        user_text: string;
        assistant_text: string;
        prompt_tokens: number | null;
        completion_tokens: number | null;
      }[]
    >("get_turns", { sessionId: sid });

    messages = [];
    let mid = 0;
    // Track the latest prompt_tokens from the most recent assistant turn
    contextTokens = 0;
    for (const t of turns) {
      messages.push({
        id: ++mid,
        role: "user",
        content: t.user_text,
        streaming: false,
      });
      messages.push({
        id: ++mid,
        role: "assistant",
        content: t.assistant_text,
        streaming: false,
      });
      if (t.prompt_tokens != null) {
        contextTokens = t.prompt_tokens;
      }
    }
    messageId = mid;
  }

  // ── Configure API key ──────────────────────────────────────
  async function configureApiKey() {
    if (!apiKey.trim()) return;
    loading = true;
    try {
      await invoke("set_api_key", { apiKey: apiKey.trim() });
      apiKeyConfigured = true;
      showSettings = false;
      await loadSessions();
    } catch (e: any) {
      alert("API Key 配置失败：" + getErrorMessage(e));
    } finally {
      loading = false;
    }
  }

  // ── Configure model ────────────────────────────────────────
  async function configureModel(newModel: string) {
    model = newModel;
    loading = true;
    try {
      await invoke("set_model", { model: newModel });
    } catch (e: any) {
      alert("模型切换失败：" + getErrorMessage(e));
    } finally {
      loading = false;
    }
  }

  // ── Clear all data ─────────────────────────────────────────
  async function clearAllData() {
    try {
      await invoke("clear_all_data");
    } catch (e: any) {
      alert("清除数据失败：" + getErrorMessage(e));
    }
  }

  // ── Sync actions ──────────────────────────────────────────
  async function createSyncDoc() {
    syncLoading = true;
    try {
      const ticket = await invoke<string>("init_and_create_sync_doc");
      syncActive = true;
      syncTicket = ticket;
    } catch (e: any) {
      alert("创建同步链失败：" + getErrorMessage(e));
    } finally {
      syncLoading = false;
    }
  }

  async function joinSyncDoc(ticket: string) {
    syncLoading = true;
    try {
      await invoke("join_sync_doc", { ticket });
      syncActive = true;
    } catch (e: any) {
      alert("加入同步链失败：" + getErrorMessage(e));
    } finally {
      syncLoading = false;
    }
  }

  async function stopSync() {
    syncLoading = true;
    try {
      await invoke("stop_sync");
      syncActive = false;
      syncTicket = "";
    } catch (e: any) {
      alert("停止同步失败：" + getErrorMessage(e));
    } finally {
      syncLoading = false;
    }
  }

  // ── Send message ───────────────────────────────────────────
  async function sendMessage() {
    const text = inputText.trim();
    if (!text || sending) return;

    // Lazily create session on first send
    if (pendingNewChat) {
      const sid = await invoke<string>("create_session");
      activeSessionId = sid;
      pendingNewChat = false;
      // Move draft from __new__ to real session id
      drafts = { ...drafts, [sid]: drafts["__new__"] ?? "" };
      const { ["__new__"]: _, ...rest } = drafts;
      drafts = rest;
      await loadSessions();
    }

    if (!activeSessionId) return;

    sending = true;
    const mid = ++messageId;
    messages.push({ id: mid, role: "user", content: text, streaming: false });

    const assistantMid = ++messageId;
    messages.push({
      id: assistantMid,
      role: "assistant",
      content: "",
      streaming: true,
    });

    inputText = "";
    // Clear draft for this session after send
    drafts = { ...drafts, [activeSessionId]: "" };

    // Listen for streaming chunks
    if (unlistenChunk) unlistenChunk();
    if (unlistenDone) unlistenDone();

    unlistenChunk = await listen<{ session_id: string; content: string }>(
      "chat-chunk",
      (event) => {
        if (event.payload.session_id === activeSessionId) {
          const msg = messages.find((m) => m.id === assistantMid);
          if (msg) {
            msg.content += event.payload.content;
            messages = [...messages]; // trigger reactivity
          }
        }
      },
    );

    unlistenDone = await listen<{ session_id: string; prompt_tokens?: number }>(
      "chat-done",
      (event) => {
        if (event.payload.session_id === activeSessionId) {
          const msg = messages.find((m) => m.id === assistantMid);
          if (msg) {
            msg.streaming = false;
            messages = [...messages];
          }
          if (event.payload.prompt_tokens != null) {
            contextTokens = event.payload.prompt_tokens;
          }
          if (unlistenChunk) unlistenChunk();
          if (unlistenDone) unlistenDone();
          sending = false;
        }
      },
    );

    try {
      await invoke("send_message", {
        sessionId: activeSessionId,
        content: text,
      });
    } catch (e: any) {
      const msg = messages.find((m) => m.id === assistantMid);
      if (msg) {
        msg.content = "错误：" + getErrorMessage(e);
        msg.streaming = false;
        messages = [...messages];
      }
      sending = false;
    }
  }

  // ── Init ───────────────────────────────────────────────────
  $effect(() => {
    invoke<boolean>("check_api_key").then((ok) => {
      apiKeyConfigured = ok;
    });
    invoke<string>("get_model").then((m) => {
      model = m;
    });
    loadSessions();
    loadKnowledgeBases();

    // Check sync status
    invoke<{ status: string; active: boolean; ticket: string | null }>("get_sync_status").then(
      (s) => {
        syncActive = s.active;
        if (s.ticket) syncTicket = s.ticket;
      },
    );

    // Listen for title updates (AI-generated or manual)
    const setupTitleListener = async () => {
      const unlisten = await listen<{ session_id: string; title: string }>(
        "session-title-updated",
        (event) => {
          const s = sessions.find(
            (s) => s.session_id === event.payload.session_id,
          );
          if (s) {
            s.title = event.payload.title;
            sessions = [...sessions];
          }
        },
      );
      return unlisten;
    };
    let unlistenTitle: UnlistenFn | null = null;
    setupTitleListener().then((fn) => (unlistenTitle = fn));

    // Listen for session deletions
    const setupDeleteListener = async () => {
      const unlisten = await listen<{ session_id: string }>(
        "session-deleted",
        (event) => {
          const sid = event.payload.session_id;
          if (activeSessionId === sid) {
            activeSessionId = null;
            pendingNewChat = false;
            messages = [];
            messageId = 0;
          }
          sessions = sessions.filter((s) => s.session_id !== sid);
        },
      );
      return unlisten;
    };
    let unlistenDelete: UnlistenFn | null = null;
    setupDeleteListener().then((fn) => (unlistenDelete = fn));

    // Listen for data-cleared
    const setupClearListener = async () => {
      const unlisten = await listen("data-cleared", () => {
        apiKeyConfigured = false;
        apiKey = "";
        model = "deepseek-v4-flash";
        showSettings = false;
        sessions = [];
        activeSessionId = null;
        pendingNewChat = false;
        messages = [];
        messageId = 0;
        contextTokens = 0;
        // keep drafts, they'll be stale but harmless
      });
      return unlisten;
    };
    let unlistenClear: UnlistenFn | null = null;
    setupClearListener().then((fn) => (unlistenClear = fn));

    // Listen for sync status changes
    const setupSyncListener = async () => {
      const unlisten = await listen<{ status: string; active: boolean }>(
        "sync-status-changed",
        async (event) => {
          syncActive = event.payload.active;
          if (event.payload.active) {
            const s = await invoke<{ status: string; active: boolean; ticket: string | null }>("get_sync_status");
            if (s.ticket) syncTicket = s.ticket;
          } else {
            syncTicket = "";
          }
        },
      );
      return unlisten;
    };
    let unlistenSync: UnlistenFn | null = null;
    setupSyncListener().then((fn) => (unlistenSync = fn));

    return () => {
      if (unlistenTitle) unlistenTitle();
      if (unlistenDelete) unlistenDelete();
      if (unlistenClear) unlistenClear();
      if (unlistenSync) unlistenSync();
    };
  });
</script>

<div class="app">
  <Sidebar
    {sessions}
    {activeSessionId}
    {sidebarCollapsed}
    onCreate={startNewChat}
    onSelect={selectSession}
    onUpdateTitle={updateSessionTitle}
    onDelete={(sid) => (sessionToDelete = sid)}
    onSettings={() => (showSettings = true)}
  />

  <!-- Mobile sidebar overlay backdrop -->
  {#if !sidebarCollapsed}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="sidebar-overlay"
      role="presentation"
      onclick={() => (sidebarCollapsed = true)}
    ></div>
  {/if}

  <!-- Main chat area -->
  <div class="chat-container">
    <button
      class="btn-toggle-sidebar"
      onclick={() => (sidebarCollapsed = !sidebarCollapsed)}
      title={sidebarCollapsed ? "打开菜单" : "关闭菜单"}
    >
      <svg
        width="20"
        height="20"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        stroke-width="2"
        stroke-linecap="round"
      >
        {#if sidebarCollapsed}
          <line x1="3" y1="6" x2="21" y2="6"></line>
          <line x1="3" y1="12" x2="21" y2="12"></line>
          <line x1="3" y1="18" x2="21" y2="18"></line>
        {:else}
          <polyline points="15 18 9 12 15 6"></polyline>
        {/if}
      </svg>
    </button>

    {#if contextTokens > 0}
      {@const pct = Math.min(100, (contextTokens / CONTEXT_WINDOW) * 100)}
      {@const color = pct > 90 ? "var(--danger)" : pct > 60 ? "#eab308" : "var(--accent)"}
      {@const r = 13}
      {@const circumference = 2 * Math.PI * r}
      {@const dashOffset = circumference * (1 - pct / 100)}
      <div class="token-ring-wrapper" title="">
        <svg
          class="token-ring"
          width="34"
          height="34"
          viewBox="0 0 34 34"
        >
          <!-- background circle -->
          <circle
            cx="17"
            cy="17"
            r={r}
            fill="none"
            stroke="var(--border-color)"
            stroke-width="3"
          />
          <!-- progress arc -->
          <circle
            cx="17"
            cy="17"
            r={r}
            fill="none"
            stroke={color}
            stroke-width="3"
            stroke-linecap="round"
            stroke-dasharray={circumference}
            stroke-dashoffset={dashOffset}
            transform="rotate(-90 17 17)"
            style="transition: stroke-dashoffset 0.4s ease, stroke 0.3s ease;"
          />
          <!-- percentage text in center -->
          <text
            x="17"
            y="18"
            text-anchor="middle"
            dominant-baseline="middle"
            fill={color}
            font-size="8"
            font-weight="600"
            style="transition: fill 0.3s ease;"
          >
            {Math.round(pct)}%
          </text>
        </svg>
        <!-- hover tooltip -->
        <div class="token-tooltip">
          <div class="token-tooltip-row">
            <span>已使用</span>
            <span>{formatNumber(contextTokens)}</span>
          </div>
          <div class="token-tooltip-row">
            <span>总容量</span>
            <span>{formatNumber(CONTEXT_WINDOW)}</span>
          </div>
          <div class="token-tooltip-row">
            <span>占比</span>
            <span>{pct.toFixed(1)}%</span>
          </div>
        </div>
      </div>
    {/if}

    {#if !apiKeyConfigured}
      <div class="apikey-warning">
        <div class="apikey-warning-card">
          <h2>欢迎使用 chatPM</h2>
          <p>请先配置您的 DeepSeek API Key 以开始使用</p>
          <button class="btn-primary-lg" onclick={() => (showSettings = true)}>
            配置 API Key
          </button>
        </div>
      </div>
    {:else if !activeSessionId && !pendingNewChat}
      <div class="empty-state">
        <div class="empty-state-content">
          <svg
            width="48"
            height="48"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            stroke-linecap="round"
            opacity="0.3"
          >
            <path
              d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"
            ></path>
          </svg>
          <h2>开始新的聊天</h2>
          <p>点击左侧「新建聊天」或选择一个已有会话</p>
        </div>
      </div>
    {:else}
      <MessageList {messages} />
      <ChatInput
        {inputText}
        {sending}
        onInputTextChange={(val: string) => (inputText = val)}
        onSend={sendMessage}
      >
        {#snippet children()}
          <KbSelector
            activeKbs={knowledgeBases.filter(k => activeKbIds.includes(k.kb_id))}
            allKbs={knowledgeBases}
            onToggle={toggleKbRef}
          />
        {/snippet}
      </ChatInput>

    {/if}
  </div>
</div>

<!-- KB management button in sidebar area -->
<button class="kb-manage-fab" onclick={() => showKbManager = true} title="管理资料库">
  <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
    <path d="M4 19.5A2.5 2.5 0 0 1 6.5 17H20"></path>
    <path d="M6.5 2H20v20H6.5A2.5 2.5 0 0 1 4 19.5v-15A2.5 2.5 0 0 1 6.5 2z"></path>
  </svg>
</button>

<KbManageModal
  show={showKbManager}
  onClose={() => { showKbManager = false; loadKnowledgeBases(); }}
/>

<SettingsModal
  show={showSettings}
  {apiKey}
  {model}
  {loading}
  {syncActive}
  {syncTicket}
  onApiKeyChange={(val: string) => (apiKey = val)}
  onModelChange={configureModel}
  onClose={() => (showSettings = false)}
  onSave={configureApiKey}
  onClear={clearAllData}
  onCreateSync={createSyncDoc}
  onJoinSync={joinSyncDoc}
  onStopSync={stopSync}
/>

<ConfirmDialog
  show={sessionToDelete !== null}
  title="删除会话"
  message="确定要删除这个会话吗？此操作无法撤销。"
  confirmText="删除"
  cancelText="取消"
  danger={true}
  onConfirm={confirmDeleteSession}
  onCancel={() => (sessionToDelete = null)}
/>

<style>
  /* ── CSS Variables (ChatGPT dark theme) ────────────────── */
  :global(body) {
    --bg-primary: #343541;
    --bg-secondary: #202123;
    --bg-surface: #40414f;
    --bg-input: #40414f;
    --bg-hover: #2b2c32;
    --bg-active: #343541;
    --border-color: #4d4d4f;
    --text-primary: #ececf1;
    --text-secondary: #c5c5d2;
    --text-muted: #8e8ea0;
    --accent: #19c37d;
    --accent-hover: #1aac6d;
    --danger: #ef4444;
    --font-sans: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto,
      "Helvetica Neue", Arial, "Noto Sans SC", "PingFang SC", "Microsoft YaHei",
      sans-serif;
    --font-mono: "SF Mono", "Cascadia Code", "Fira Code", monospace;
    --radius: 8px;
    --radius-lg: 12px;
    margin: 0;
    padding: 0;
    font-family: var(--font-sans);
    background: var(--bg-primary);
    color: var(--text-primary);
  }

  *,
  *::before,
  *::after {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  /* ── App Layout ────────────────────────────────────────── */
  .app {
    display: flex;
    height: 100vh;
    overflow: hidden;
  }

  /* Mobile sidebar overlay */
  .sidebar-overlay {
    display: none;
  }

  /* ── Chat Container ────────────────────────────────────── */
  .chat-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    position: relative;
    min-width: 0;
    overflow-x: hidden;
  }

  /* ── Sidebar toggle ──────────────────────────────────── */
  .btn-toggle-sidebar {
    position: absolute;
    top: 10px;
    left: 10px;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    background: var(--bg-secondary);
    color: var(--text-muted);
    cursor: pointer;
    transition:
      color 0.15s,
      background 0.15s;
  }

  .btn-toggle-sidebar:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  /* ── API Key Warning ───────────────────────────────────── */
  .apikey-warning {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .apikey-warning-card {
    text-align: center;
    max-width: 400px;
    padding: 40px;
  }

  .apikey-warning-card h2 {
    font-size: 24px;
    margin-bottom: 12px;
    color: var(--text-primary);
  }

  .apikey-warning-card p {
    color: var(--text-muted);
    margin-bottom: 24px;
    font-size: 15px;
  }

  .btn-primary-lg {
    padding: 12px 28px;
    border: none;
    border-radius: var(--radius);
    background: var(--accent);
    color: #fff;
    font-size: 15px;
    font-weight: 600;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-primary-lg:hover {
    background: var(--accent-hover);
  }

  /* ── Empty State ───────────────────────────────────────── */
  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .empty-state-content {
    text-align: center;
  }

  .empty-state-content h2 {
    font-size: 20px;
    color: var(--text-secondary);
    margin: 16px 0 8px;
  }

  .empty-state-content p {
    color: var(--text-muted);
    font-size: 14px;
  }

  /* ── Markdown body styles (global for {@html}) ───────── */
  :global {
    .markdown-body > *:first-child {
      margin-top: 0;
    }

    .markdown-body > *:last-child {
      margin-bottom: 0;
    }

    .markdown-body p {
      margin: 0.5em 0;
      white-space: pre-wrap;
    }

    .markdown-body h1,
    .markdown-body h2,
    .markdown-body h3,
    .markdown-body h4,
    .markdown-body h5,
    .markdown-body h6 {
      margin: 1em 0 0.5em;
      font-weight: 600;
      line-height: 1.4;
    }

    .markdown-body h1 {
      font-size: 1.5em;
    }
    .markdown-body h2 {
      font-size: 1.3em;
    }
    .markdown-body h3 {
      font-size: 1.15em;
    }
    .markdown-body h4 {
      font-size: 1em;
    }

    .markdown-body ul,
    .markdown-body ol {
      margin: 0.5em 0;
      padding-left: 1.5em;
    }

    .markdown-body li {
      margin: 0.2em 0;
    }

    .markdown-body code {
      background: rgba(0, 0, 0, 0.3);
      padding: 2px 6px;
      border-radius: 4px;
      font-family: var(--font-mono);
      font-size: 0.9em;
    }

    .markdown-body pre {
      margin: 0.8em 0;
      padding: 12px 16px;
      background: #1e1e2e;
      border: 1px solid var(--border-color);
      border-radius: var(--radius);
      overflow-x: auto;
      max-width: 100%;
    }

    .markdown-body pre code {
      background: none;
      padding: 0;
      border-radius: 0;
      font-size: 0.85em;
      line-height: 1.6;
      color: #cdd6f4;
      white-space: pre;
      word-break: normal;
    }

    .markdown-body blockquote {
      margin: 0.8em 0;
      padding: 4px 12px;
      border-left: 3px solid var(--accent);
      color: var(--text-secondary);
      background: rgba(255, 255, 255, 0.03);
      border-radius: 0 4px 4px 0;
    }

    .markdown-body blockquote p {
      margin: 0.3em 0;
    }

    .markdown-body table {
      margin: 0.8em 0;
      border-collapse: collapse;
      width: 100%;
      max-width: 100%;
      font-size: 0.9em;
      display: block;
      overflow-x: auto;
    }

    .markdown-body th,
    .markdown-body td {
      padding: 8px 12px;
      border: 1px solid var(--border-color);
      text-align: left;
    }

    .markdown-body th {
      background: var(--bg-surface);
      font-weight: 600;
    }

    .markdown-body tr:nth-child(even) {
      background: rgba(255, 255, 255, 0.02);
    }

    .markdown-body a {
      color: var(--accent);
      text-decoration: underline;
    }

    .markdown-body hr {
      margin: 1em 0;
      border: none;
      border-top: 1px solid var(--border-color);
    }

    .markdown-body img {
      max-width: 100%;
      border-radius: var(--radius);
    }

    .markdown-body strong {
      font-weight: 700;
    }
  }

  /* ── Scrollbar ──────────────────────────────────────────── */
  ::-webkit-scrollbar {
    width: 6px;
  }

  ::-webkit-scrollbar-track {
    background: transparent;
  }

  ::-webkit-scrollbar-thumb {
    background: var(--border-color);
    border-radius: 3px;
  }

  ::-webkit-scrollbar-thumb:hover {
    background: var(--text-muted);
  }

  /* ── Mobile Responsive ──────────────────────────────────── */
  @media (max-width: 768px) {
    /* Sidebar overlay */
    .sidebar-overlay {
      display: block;
      position: fixed;
      inset: 0;
      background: rgba(0, 0, 0, 0.5);
      z-index: 55;
    }

    /* API key warning */
    .apikey-warning-card {
      padding: 24px 16px;
    }

    .apikey-warning-card h2 {
      font-size: 20px;
    }

    /* Empty state */
    .empty-state-content h2 {
      font-size: 18px;
    }
  }

  /* Very small screens */
  @media (max-width: 480px) {
    .btn-toggle-sidebar {
      top: 6px;
      left: 6px;
      width: 30px;
      height: 30px;
    }

    .token-ring-wrapper {
      top: 6px;
      right: 6px;
      width: 30px;
      height: 30px;
    }
  }

  /* ── Token Ring ────────────────────────────────────────── */

  .token-ring-wrapper {
    position: absolute;
    top: 10px;
    right: 10px;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 34px;
    height: 34px;
    border-radius: 50%;
    cursor: pointer;
  }

  .token-ring-wrapper:hover {
    background: var(--bg-hover);
  }

  .token-ring {
    display: block;
  }

  /* Tooltip */
  .token-tooltip {
    position: absolute;
    top: calc(100% + 8px);
    right: 0;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    padding: 10px 14px;
    white-space: nowrap;
    pointer-events: none;
    opacity: 0;
    transition: opacity 0.15s ease;
    z-index: 20;
    box-shadow: 0 4px 16px rgba(0, 0, 0, 0.3);
  }

  .token-ring-wrapper:hover .token-tooltip {
    opacity: 1;
  }

  .token-tooltip-row {
    display: flex;
    justify-content: space-between;
    gap: 16px;
    font-size: 12px;
    line-height: 1.8;
    color: var(--text-secondary);
  }

  .token-tooltip-row span:last-child {
    font-family: var(--font-mono);
    color: var(--text-primary);
    font-weight: 500;
  }

  .kb-manage-fab {
    position: fixed;
    bottom: 100px;
    right: 24px;
    width: 40px;
    height: 40px;
    border: 1px solid var(--border-color);
    border-radius: 50%;
    background: var(--bg-sidebar);
    color: var(--text-muted);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
    transition:
      color 0.15s,
      border-color 0.15s,
      transform 0.15s;
    z-index: 50;
  }

  .kb-manage-fab:hover {
    color: var(--accent);
    border-color: var(--accent);
    transform: scale(1.05);
  }
</style>
