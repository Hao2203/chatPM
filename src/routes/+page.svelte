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

  let sessions = $state<SessionInfo[]>([]);
  let activeSessionId = $state<string | null>(null);
  let messages = $state<Message[]>([]);
  let inputText = $state("");
  let messageId = $state(0);

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
      messages.push({ id: ++mid, role: "user", content: t.user_text, streaming: false });
      messages.push({ id: ++mid, role: "assistant", content: t.assistant_text, streaming: false });
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
    if (!text || !activeSessionId) return;

    const mid = ++messageId;
    messages.push({ id: mid, role: "user", content: text, streaming: false });

    const assistantMid = ++messageId;
    messages.push({ id: assistantMid, role: "assistant", content: "", streaming: true });

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

    unlistenDone = await listen<{ session_id: string }>("chat-done", (event) => {
      if (event.payload.session_id === activeSessionId) {
        const msg = messages.find((m) => m.id === assistantMid);
        if (msg) {
          msg.streaming = false;
          messages = [...messages];
        }
        if (unlistenChunk) unlistenChunk();
        if (unlistenDone) unlistenDone();
      }
    });

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
    }
  }

  let isComposing = $state(false);

  function onInput(e: Event) {
    if (!isComposing) {
      inputText = (e.target as HTMLTextAreaElement).value;
    }
  }

  function onCompositionStart() {
    isComposing = true;
  }

  function onCompositionEnd(e: CompositionEvent) {
    isComposing = false;
    inputText = (e.target as HTMLTextAreaElement).value;
  }

  function handleKeydown(e: KeyboardEvent) {
    // Don't intercept Enter during IME composition
    if (isComposing || e.isComposing || e.keyCode === 229) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  }

  // ── Init ───────────────────────────────────────────────────
  $effect(() => {
    // Check if API key is already configured from previous session
    invoke<boolean>("check_api_key").then((ok) => {
      apiKeyConfigured = ok;
    });
    loadSessions();
  });
</script>

<div class="app {showSettings ? 'settings-open' : ''}">
  <!-- Sidebar -->
  <aside class="sidebar">
    <div class="sidebar-header">
      <h1>chatPM</h1>
      <div class="sidebar-actions">
        <button class="btn btn-icon" onclick={createSession} title="新建会话">+</button>
        <button class="btn btn-icon" onclick={() => (showSettings = true)} title="设置">&#9881;</button>
      </div>
    </div>

    <div class="session-list">
      {#if sessions.length === 0}
        <div class="empty-hint">暂无会话，点击 + 创建</div>
      {:else}
        {#each sessions as s}
          <button
            class="session-item"
            class:active={s.session_id === activeSessionId}
            onclick={() => selectSession(s.session_id)}
          >
            <span class="session-id">{s.session_id.slice(0, 8)}...</span>
            <span class="session-time">{new Date(s.created_at).toLocaleDateString()}</span>
          </button>
        {/each}
      {/if}
    </div>
  </aside>

  <!-- Main chat area -->
  <main class="chat-area">
    {#if !activeSessionId}
      <div class="empty-state">
        <p>选择一个会话或创建新会话开始聊天</p>
      </div>
    {:else}
      <div class="messages">
        {#each messages as msg (msg.id)}
          <div class="message" class:user={msg.role === "user"} class:assistant={msg.role === "assistant"}>
            <div class="bubble">
              {msg.content}
              {#if msg.streaming}
                <span class="cursor">|</span>
              {/if}
            </div>
          </div>
        {/each}
      </div>

      <div class="input-area">
        {#if !apiKeyConfigured}
          <div class="apikey-hint">
            请先<a href="#" onclick={(e) => { e.preventDefault(); showSettings = true; }}>配置 API Key</a>后再发送消息
          </div>
        {:else}
          <textarea
            value={inputText}
            oninput={onInput}
            oncompositionstart={onCompositionStart}
            oncompositionend={onCompositionEnd}
            onkeydown={handleKeydown}
            placeholder="输入消息... (Enter 发送, Shift+Enter 换行)"
            rows="1"
          ></textarea>
          <button class="btn btn-send" onclick={sendMessage} disabled={!inputText.trim()}>
            发送
          </button>
        {/if}
      </div>
    {/if}
  </main>
</div>

<!-- Settings panel -->
{#if showSettings}
  <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
  <div class="settings-overlay" onclick={() => (showSettings = false)}>
    <!-- svelte-ignore a11y_click_events_have_key_events a11y_no_static_element_interactions -->
    <div class="settings-panel" onclick={(e) => e.stopPropagation()}>
      <h2>设置</h2>
      <label>
        DeepSeek API Key
        <input
          type="password"
          bind:value={apiKey}
          placeholder="sk-..."
          disabled={loading}
        />
      </label>
      <div class="settings-actions">
        <button class="btn btn-primary" onclick={configureApiKey} disabled={loading}>
          {loading ? "配置中..." : "保存"}
        </button>
        <button class="btn" onclick={() => (showSettings = false)}>取消</button>
      </div>
    </div>
  </div>
{/if}

<style>
  /* ── Reset & Base ─────────────────────────────────────── */
  *,
  *::before,
  *::after {
    box-sizing: border-box;
    margin: 0;
    padding: 0;
  }

  :global(body) {
    font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: #1a1a2e;
    color: #e0e0e0;
    overflow: hidden;
  }

  .app {
    display: flex;
    height: 100vh;
  }

  /* ── Sidebar ──────────────────────────────────────────── */
  .sidebar {
    width: 260px;
    background: #16213e;
    display: flex;
    flex-direction: column;
    border-right: 1px solid #0f3460;
  }

  .sidebar-header {
    padding: 16px;
    display: flex;
    align-items: center;
    justify-content: space-between;
    border-bottom: 1px solid #0f3460;
  }

  .sidebar-header h1 {
    font-size: 18px;
    font-weight: 700;
    color: #e94560;
  }

  .sidebar-actions {
    display: flex;
    gap: 4px;
  }

  .session-list {
    flex: 1;
    overflow-y: auto;
    padding: 8px;
  }

  .session-item {
    display: flex;
    flex-direction: column;
    gap: 2px;
    width: 100%;
    padding: 10px 12px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: #ccc;
    cursor: pointer;
    text-align: left;
    margin-bottom: 4px;
    transition: background 0.15s;
  }

  .session-item:hover {
    background: #1a1a40;
  }

  .session-item.active {
    background: #0f3460;
    color: #fff;
  }

  .session-id {
    font-size: 13px;
    font-family: monospace;
  }

  .session-time {
    font-size: 11px;
    color: #888;
  }

  .empty-hint {
    padding: 20px 12px;
    color: #666;
    font-size: 13px;
    text-align: center;
  }

  /* ── Chat Area ─────────────────────────────────────────── */
  .chat-area {
    flex: 1;
    display: flex;
    flex-direction: column;
    background: #1a1a2e;
  }

  .empty-state {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    color: #555;
    font-size: 16px;
  }

  .messages {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
    display: flex;
    flex-direction: column;
    gap: 16px;
  }

  .message {
    display: flex;
    max-width: 80%;
  }

  .message.user {
    align-self: flex-end;
  }

  .message.assistant {
    align-self: flex-start;
  }

  .bubble {
    padding: 10px 16px;
    border-radius: 12px;
    font-size: 14px;
    line-height: 1.6;
    white-space: pre-wrap;
    word-break: break-word;
  }

  .user .bubble {
    background: #0f3460;
    color: #e0e0e0;
    border-bottom-right-radius: 4px;
  }

  .assistant .bubble {
    background: #16213e;
    color: #d0d0d0;
    border-bottom-left-radius: 4px;
  }

  .cursor {
    animation: blink 1s step-end infinite;
    color: #e94560;
    font-weight: bold;
  }

  @keyframes blink {
    50% {
      opacity: 0;
    }
  }

  /* ── Input Area ────────────────────────────────────────── */
  .input-area {
    padding: 16px 20px;
    border-top: 1px solid #0f3460;
    display: flex;
    gap: 10px;
    align-items: flex-end;
  }

  .input-area textarea {
    flex: 1;
    resize: none;
    padding: 10px 14px;
    border-radius: 8px;
    border: 1px solid #0f3460;
    background: #16213e;
    color: #e0e0e0;
    font-size: 14px;
    font-family: inherit;
    outline: none;
    max-height: 120px;
  }

  .input-area textarea:focus {
    border-color: #e94560;
  }

  .apikey-hint {
    flex: 1;
    padding: 12px 16px;
    border-radius: 8px;
    background: #2a1a1a;
    border: 1px solid #e9456040;
    color: #e94560;
    font-size: 14px;
    text-align: center;
  }

  .apikey-hint a {
    color: #e94560;
    font-weight: 600;
    text-decoration: underline;
  }

  /* ── Buttons ───────────────────────────────────────────── */
  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 8px;
    background: #16213e;
    color: #ccc;
    cursor: pointer;
    font-size: 14px;
    transition: background 0.15s;
  }

  .btn:hover {
    background: #0f3460;
  }

  .btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .btn-icon {
    width: 36px;
    height: 36px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 20px;
    border-radius: 50%;
  }

  .btn-send {
    background: #e94560;
    color: #fff;
    font-weight: 600;
  }

  .btn-send:hover {
    background: #d63850;
  }

  .btn-primary {
    background: #e94560;
    color: #fff;
    font-weight: 600;
  }

  /* ── Settings Overlay ──────────────────────────────────── */
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
    background: #16213e;
    border: 1px solid #0f3460;
    border-radius: 12px;
    padding: 28px;
    width: 400px;
    max-width: 90vw;
  }

  .settings-panel h2 {
    margin-bottom: 20px;
    color: #e94560;
  }

  .settings-panel label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 14px;
    color: #aaa;
    margin-bottom: 20px;
  }

  .settings-panel input {
    padding: 10px 14px;
    border-radius: 8px;
    border: 1px solid #0f3460;
    background: #1a1a2e;
    color: #e0e0e0;
    font-size: 14px;
    outline: none;
  }

  .settings-panel input:focus {
    border-color: #e94560;
  }

  .settings-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }
</style>
