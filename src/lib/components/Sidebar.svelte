<script lang="ts">
  interface SessionInfo {
    session_id: string;
    created_at: string;
    title: string | null;
  }

  let {
    sessions = [],
    activeSessionId = null,
    sidebarCollapsed = false,
    onCreate = () => {},
    onSelect = (_sid: string) => {},
    onUpdateTitle = (_sid: string, _title: string) => {},
    onDelete = (_sid: string) => {},
    onSettings = () => {},
  }: {
    sessions: SessionInfo[];
    activeSessionId: string | null;
    sidebarCollapsed: boolean;
    onCreate: () => void;
    onSelect: (sid: string) => void;
    onUpdateTitle: (sid: string, title: string) => void;
    onDelete: (sid: string) => void;
    onSettings: () => void;
  } = $props();

  let editingSessionId = $state<string | null>(null);
  let editTitle = $state("");
  let editInputEl = $state<HTMLInputElement | null>(null);

  $effect(() => {
    editInputEl?.focus();
  });

  function startEdit(sessionId: string, currentTitle: string | null) {
    editingSessionId = sessionId;
    editTitle = currentTitle ?? "";
  }

  function commitEdit() {
    if (editingSessionId && editTitle.trim()) {
      onUpdateTitle(editingSessionId, editTitle.trim());
    }
    editingSessionId = null;
    editTitle = "";
  }

  function cancelEdit() {
    editingSessionId = null;
    editTitle = "";
  }

  function handleEditKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      commitEdit();
    } else if (e.key === "Escape") {
      cancelEdit();
    }
  }

  function sessionLabel(s: SessionInfo): string {
    return s.title ?? s.session_id.slice(0, 12) + "...";
  }

  function formatTime(dateStr: string): string {
    const d = new Date(dateStr);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    if (diff < 86400000) {
      return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    }
    return d.toLocaleDateString([], { month: "short", day: "numeric" });
  }
</script>

<aside class="sidebar" class:collapsed={sidebarCollapsed}>
  <div class="sidebar-inner">
    <div class="sidebar-header">
      <button class="btn-new-chat" onclick={onCreate} title="新建会话">
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
    </div>

    <div class="session-list">
      {#if sessions.length === 0}
        <div class="empty-hint">暂无聊天记录</div>
      {:else}
        {#each sessions as s}
          <button
            class="session-item"
            class:active={s.session_id === activeSessionId}
            onclick={() => onSelect(s.session_id)}
            ondblclick={() => startEdit(s.session_id, s.title)}
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
            {#if editingSessionId === s.session_id}
              <input
                class="title-edit-input"
                type="text"
                bind:value={editTitle}
                bind:this={editInputEl}
                onkeydown={handleEditKeydown}
                onblur={commitEdit}
              />
            {:else}
              <span class="session-label">{sessionLabel(s)}</span>
            {/if}
            <span class="session-time">{formatTime(s.created_at)}</span>
            <span
              class="btn-delete"
              title="删除会话"
              role="button"
              tabindex="0"
              onclick={(e) => {
                e.stopPropagation();
                onDelete(s.session_id);
              }}
              onkeydown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  e.stopPropagation();
                  onDelete(s.session_id);
                }
              }}
            >
              <svg
                width="14"
                height="14"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
              >
                <polyline points="3 6 5 6 21 6"></polyline>
                <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
                <path d="M10 11v6"></path>
                <path d="M14 11v6"></path>
                <path d="M9 6V4a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2"></path>
              </svg>
            </span>
          </button>
        {/each}
      {/if}
    </div>

    <div class="sidebar-footer">
      <button class="btn-settings" onclick={onSettings} title="设置">
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

<style>
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
    z-index: 50;
  }

  .sidebar.collapsed {
    width: 0;
    min-width: 0;
  }

  .sidebar-inner {
    width: 260px;
    min-width: 260px;
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 8px;
    overflow: hidden;
  }

  .sidebar-header {
    padding-bottom: 12px;
    border-bottom: 1px solid var(--border-color);
  }

  .btn-new-chat {
    display: flex;
    align-items: center;
    gap: 10px;
    width: 95%;
    padding: 10px 14px;
    border: 1px solid var(--border-color);
    border-radius: var(--radius);
    background: transparent;
    color: var(--text-primary);
    font-size: 14px;
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }

  .btn-new-chat:hover {
    background: var(--bg-hover);
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
    font-size: 13px;
  }

  .title-edit-input {
    flex: 1;
    min-width: 0;
    padding: 2px 6px;
    border: 1px solid var(--accent);
    border-radius: 4px;
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 13px;
    outline: none;
  }

  .btn-delete {
    display: flex;
    align-items: center;
    justify-content: center;
    width: 28px;
    height: 28px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-muted);
    cursor: pointer;
    opacity: 0;
    transition:
      opacity 0.15s,
      color 0.15s,
      background 0.15s;
    flex-shrink: 0;
  }

  .session-item:hover .btn-delete {
    opacity: 1;
  }

  .btn-delete:hover {
    color: var(--danger);
    background: rgba(239, 68, 68, 0.1);
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
    white-space: nowrap;
  }

  .btn-settings:hover {
    background: var(--bg-hover);
  }

  @media (max-width: 768px) {
    .sidebar {
      position: fixed;
      top: 0;
      left: 0;
      height: 100vh;
      width: 280px;
      min-width: 280px;
      z-index: 60;
      box-shadow: 4px 0 24px rgba(0, 0, 0, 0.4);
    }

    .sidebar.collapsed {
      width: 0;
      min-width: 0;
      box-shadow: none;
    }

    .sidebar-inner {
      width: 280px;
      min-width: 280px;
    }
  }
</style>
