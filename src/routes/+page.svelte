<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
  }

  // ── State ──────────────────────────────────────────────────
  let apiKey = $state("");
  let apiKeyConfigured = $state(false);
  let showSettings = $state(false);
  let loading = $state(false);
  let sidebarCollapsed = $state(false);

  let sessions = $state<SessionInfo[]>([]);
  let activeSessionId = $state<string | null>(null);
  let messages = $state<Message[]>([]);
  let inputText = $state("");
  let messageId = $state(0);
  let sending = $state(false);

  // ── Event cleanup ──────────────────────────────────────────
  let unlistenChunk: UnlistenFn | null = null;
  let unlistenDone: UnlistenFn | null = null;

  // ── Load sessions ──────────────────────────────────────────
  async function loadSessions() {
    sessions = await invoke<SessionInfo[]>("list_sessions");
  }

  // ── Create session ─────────────────────────────────────────
  async function createSession() {
    const sid = await invoke<string>("create_session");
    await loadSessions();
    activeSessionId = sid;
    messages = [];
  }

  // ── Select session ─────────────────────────────────────────
  async function selectSession(sid: string) {
    activeSessionId = sid;
    const turns = await invoke<
      { turn_num: number; user_text: string; assistant_text: string }[]
    >("get_turns", { sessionId: sid });

    messages = [];
    let mid = 0;
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
      alert("API Key 配置失败：" + e);
    } finally {
      loading = false;
    }
  }

  // ── Send message ───────────────────────────────────────────
  async function sendMessage() {
    const text = inputText.trim();
    if (!text || !activeSessionId || sending) return;

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

    unlistenDone = await listen<{ session_id: string }>(
      "chat-done",
      (event) => {
        if (event.payload.session_id === activeSessionId) {
          const msg = messages.find((m) => m.id === assistantMid);
          if (msg) {
            msg.streaming = false;
            messages = [...messages];
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
        msg.content = "错误：" + e;
        msg.streaming = false;
        messages = [...messages];
      }
      sending = false;
    }
  }

  // ── Handle Enter key (Shift+Enter for newline, Enter to send) ──
  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }

  // ── Auto-resize textarea ───────────────────────────────────
  function autoResize(e: Event) {
    const ta = e.target as HTMLTextAreaElement;
    ta.style.height = "auto";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }

  // ── Format time for session list ───────────────────────────
  function formatTime(dateStr: string): string {
    const d = new Date(dateStr);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    if (diff < 86400000) {
      return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    }
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  }

  // ── Init ───────────────────────────────────────────────────
  $effect(() => {
    invoke<boolean>("check_api_key").then((ok) => {
      apiKeyConfigured = ok;
    });
    loadSessions();
  });
</script>

<div class="app">
  <!-- Sidebar -->
  <aside class="sidebar" class:collapsed={sidebarCollapsed}>
    <div class="sidebar-inner">
      <div class="sidebar-header">
        <button class="btn-new-chat" onclick={createSession} title="新建会话">
          <svg
            width="20"
            height="20"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          >
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
          </svg>
          <span>新建聊天</span>
        </button>
        <button
          class="btn-toggle-sidebar"
          onclick={() => (sidebarCollapsed = !sidebarCollapsed)}
          title="收起侧栏"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          >
            <polyline points="15 18 9 12 15 6"></polyline>
          </svg>
        </button>
      </div>

      <div class="session-list">
        {#if sessions.length === 0}
          <div class="empty-hint">暂无聊天记录</div>
        {:else}
          {#each sessions as s}
            <button
              class="session-item"
              class:active={s.session_id === activeSessionId}
              onclick={() => selectSession(s.session_id)}
            >
              <svg
                class="session-icon"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
              >
                <path
                  d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"
                ></path>
              </svg>
              <span class="session-label">{s.session_id.slice(0, 12)}...</span>
              <span class="session-time">{formatTime(s.created_at)}</span>
            </button>
          {/each}
        {/if}
      </div>

      <div class="sidebar-footer">
        <button
          class="btn-settings"
          onclick={() => (showSettings = true)}
          title="设置"
        >
          <svg
            width="18"
            height="18"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="2"
            stroke-linecap="round"
          >
            <circle cx="12" cy="12" r="3"></circle>
            <path
              d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"
            ></path>
          </svg>
          <span>设置</span>
        </button>
      </div>
    </div>
  </aside>

  <!-- Main chat area -->
  <div class="chat-container">
    {#if sidebarCollapsed}
      <button
        class="btn-expand-sidebar"
        onclick={() => (sidebarCollapsed = false)}
        title="展开侧栏"
      >
        <svg
          width="18"
          height="18"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
        >
          <polyline points="9 18 15 12 9 6"></polyline>
        </svg>
      </button>
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
    {:else if !activeSessionId}
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
      <!-- Messages -->
      <div class="messages">
        {#each messages as msg (msg.id)}
          <div
            class="message-row"
            class:user-row={msg.role === "user"}
            class:assistant-row={msg.role === "assistant"}
          >
            <div class="message-avatar">
              {#if msg.role === "user"}
                <div class="avatar user-avatar">
                  <svg
                    width="18"
                    height="18"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                  >
                    <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"></path>
                    <circle cx="12" cy="7" r="4"></circle>
                  </svg>
                </div>
              {:else}
                <div class="avatar assistant-avatar">
                  <svg
                    width="18"
                    height="18"
                    viewBox="0 0 24 24"
                    fill="none"
                    stroke="currentColor"
                    stroke-width="2"
                    stroke-linecap="round"
                  >
                    <path d="M12 2L2 7l10 5 10-5-10-5z"></path>
                    <path d="M2 17l10 5 10-5"></path>
                    <path d="M2 12l10 5 10-5"></path>
                  </svg>
                </div>
              {/if}
            </div>
            <div class="message-content">
              <div class="message-bubble">
                <pre>{msg.content}</pre>
                {#if msg.streaming}
                  <span class="cursor">|</span>
                {/if}
              </div>
            </div>
          </div>
        {/each}

        <!-- Bottom scroll anchor -->
        <div class="scroll-anchor"></div>
      </div>

      <!-- Input area -->
      <div class="input-bar">
        <div class="input-wrapper">
          <textarea
            bind:value={inputText}
            onkeydown={handleKeydown}
            oninput={autoResize}
            placeholder="发送消息..."
            rows="1"
            disabled={sending}
          ></textarea>
          <button
            class="btn-send"
            onclick={sendMessage}
            disabled={!inputText.trim() || sending}
            title="发送"
          >
            {#if sending}
              <svg
                class="spin"
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2.5"
                stroke-linecap="round"
              >
                <circle cx="12" cy="12" r="10"></circle>
                <path d="M12 2a10 10 0 0 1 10 10" opacity="0.5"></path>
              </svg>
            {:else}
              <svg
                width="18"
                height="18"
                viewBox="0 0 24 24"
                fill="currentColor"
              >
                <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z"></path>
              </svg>
            {/if}
          </button>
        </div>
        <p class="input-hint">Enter 发送，Shift + Enter 换行</p>
      </div>
    {/if}
  </div>
</div>

<!-- Settings overlay -->
{#if showSettings}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="settings-overlay"
    role="presentation"
    onclick={() => (showSettings = false)}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="settings-panel"
      role="dialog"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="settings-header">
        <h2>设置</h2>
        <button
          class="btn-close"
          onclick={() => (showSettings = false)}
          title="关闭"
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
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
          </svg>
        </button>
      </div>
      <label class="setting-label">
        <span>DeepSeek API Key</span>
        <input
          type="password"
          bind:value={apiKey}
          placeholder="sk-..."
          disabled={loading}
        />
      </label>
      <div class="settings-actions">
        <button
          class="btn-primary"
          onclick={configureApiKey}
          disabled={loading}
        >
          {loading ? "配置中..." : "保存"}
        </button>
        <button class="btn-cancel" onclick={() => (showSettings = false)}
          >取消</button
        >
      </div>
    </div>
  </div>
{/if}

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

  /* ── Sidebar ───────────────────────────────────────────── */
  .sidebar {
    width: 260px;
    min-width: 260px;
    background: var(--bg-secondary);
    display: flex;
    flex-direction: column;
    transition:
      width 0.2s ease,
      min-width 0.2s ease;
    overflow: hidden;
  }

  .sidebar.collapsed {
    width: 0;
    min-width: 0;
  }

  .sidebar-inner {
    width: 260px;
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 8px;
  }

  .sidebar-header {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
  }

  .btn-new-chat {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 10px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-primary);
    font-size: 14px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-new-chat:hover {
    background: var(--bg-hover);
  }

  .btn-toggle-sidebar {
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 6px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    align-self: flex-end;
    transition: color 0.15s;
  }

  .btn-toggle-sidebar:hover {
    color: var(--text-primary);
  }

  /* Session list */
  .session-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }

  .empty-hint {
    padding: 20px 12px;
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
  }

  .session-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-secondary);
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    transition: background 0.15s;
    position: relative;
  }

  .session-item:hover {
    background: var(--bg-hover);
  }

  .session-item.active {
    background: var(--bg-surface);
    color: var(--text-primary);
  }

  .session-icon {
    flex-shrink: 0;
    opacity: 0.5;
  }

  .session-item.active .session-icon {
    opacity: 1;
  }

  .session-label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-family: var(--font-mono);
    font-size: 12px;
  }

  .session-time {
    font-size: 11px;
    color: var(--text-muted);
    flex-shrink: 0;
  }

  /* Sidebar footer */
  .sidebar-footer {
    padding-top: 8px;
    border-top: 1px solid var(--border-color);
  }

  .btn-settings {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 10px 14px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-secondary);
    font-size: 14px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-settings:hover {
    background: var(--bg-hover);
  }

  /* ── Chat Container ────────────────────────────────────── */
  .chat-container {
    flex: 1;
    display: flex;
    flex-direction: column;
    position: relative;
    min-width: 0;
  }

  .btn-expand-sidebar {
    position: absolute;
    top: 12px;
    left: 12px;
    z-index: 10;
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    background: var(--bg-secondary);
    color: var(--text-muted);
    cursor: pointer;
    transition:
      color 0.15s,
      background 0.15s;
  }

  .btn-expand-sidebar:hover {
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

  /* ── Messages ──────────────────────────────────────────── */
  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 20px 0 0;
    display: flex;
    flex-direction: column;
  }

  .message-row {
    display: flex;
    gap: 14px;
    padding: 16px 40px;
    width: 100%;
    max-width: 820px;
    margin: 0 auto;
  }

  .user-row {
    background: var(--bg-primary);
  }

  .assistant-row {
    background: var(--bg-secondary);
  }

  .message-avatar {
    flex-shrink: 0;
    width: 32px;
  }

  .avatar {
    width: 30px;
    height: 30px;
    border-radius: 4px;
    display: flex;
    align-items: center;
    justify-content: center;
  }

  .user-avatar {
    background: #5533aa;
    color: #fff;
  }

  .assistant-avatar {
    background: var(--accent);
    color: #fff;
  }

  .message-content {
    flex: 1;
    min-width: 0;
  }

  .message-bubble {
    line-height: 1.7;
    font-size: 15px;
    color: var(--text-primary);
  }

  .message-bubble pre {
    white-space: pre-wrap;
    word-break: break-word;
    font-family: var(--font-sans);
    line-height: 1.7;
  }

  .cursor {
    display: inline-block;
    width: 2px;
    height: 16px;
    background: var(--text-primary);
    animation: blink 1s step-end infinite;
    vertical-align: text-bottom;
    margin-left: 2px;
    border-radius: 1px;
  }

  @keyframes blink {
    50% {
      opacity: 0;
    }
  }

  .scroll-anchor {
    height: 1px;
  }

  /* ── Input Bar ─────────────────────────────────────────── */
  .input-bar {
    padding: 12px 40px 20px;
    max-width: 820px;
    width: 100%;
    margin: 0 auto;
  }

  .input-wrapper {
    display: flex;
    align-items: flex-end;
    gap: 8px;
    background: var(--bg-input);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    padding: 8px 8px 8px 16px;
    transition: border-color 0.15s;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.15);
  }

  .input-wrapper:focus-within {
    border-color: var(--accent);
  }

  .input-wrapper textarea {
    flex: 1;
    border: none;
    background: transparent;
    color: var(--text-primary);
    font-size: 15px;
    font-family: var(--font-sans);
    line-height: 1.5;
    outline: none;
    resize: none;
    padding: 4px 0;
    max-height: 200px;
  }

  .input-wrapper textarea::placeholder {
    color: var(--text-muted);
  }

  .input-wrapper textarea:disabled {
    opacity: 0.6;
  }

  .btn-send {
    flex-shrink: 0;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: #fff;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition:
      background 0.15s,
      opacity 0.15s;
  }

  .btn-send:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-send:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .input-hint {
    text-align: center;
    font-size: 11px;
    color: var(--text-muted);
    margin-top: 8px;
  }

  .spin {
    animation: spin 1s linear infinite;
  }

  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }

  /* ── Settings ──────────────────────────────────────────── */
  .settings-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 100;
  }

  .settings-panel {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    padding: 24px;
    width: 440px;
    max-width: 90vw;
  }

  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 20px;
  }

  .settings-header h2 {
    font-size: 18px;
    font-weight: 600;
  }

  .btn-close {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 32px;
    height: 32px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    transition:
      color 0.15s,
      background 0.15s;
  }

  .btn-close:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  .setting-label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 20px;
  }

  .setting-label input {
    padding: 10px 14px;
    border-radius: var(--radius);
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 14px;
    font-family: var(--font-mono);
    outline: none;
    transition: border-color 0.15s;
  }

  .setting-label input:focus {
    border-color: var(--accent);
  }

  .setting-label input:disabled {
    opacity: 0.5;
  }

  .settings-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }

  .btn-primary {
    padding: 10px 20px;
    border: none;
    border-radius: var(--radius);
    background: var(--accent);
    color: #fff;
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    transition:
      background 0.15s,
      opacity 0.15s;
  }

  .btn-primary:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-primary:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-cancel {
    padding: 10px 20px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-secondary);
    font-size: 14px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
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
</style>
