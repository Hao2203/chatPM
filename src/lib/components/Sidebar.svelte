<script lang="ts">
  interface SessionInfo {
    session_id: string;
    created_at: string;
  }

  let {
    sessions = [],
    activeSessionId = null,
    sidebarCollapsed = false,
    oncreate = () => {},
    onselect = (_sid: string) => {},
    onsettings = () => {},
  }: {
    sessions: SessionInfo[];
    activeSessionId: string | null;
    sidebarCollapsed: boolean;
    oncreate: () => void;
    onselect: (sid: string) => void;
    onsettings: () => void;
  } = $props();

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
      <button class="btn-new-chat" onclick={oncreate} title="新建会话">
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
            onclick={() => onselect(s.session_id)}
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
      <button class="btn-settings" onclick={onsettings} title="设置">
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
    width: 100%;
    height: 100%;
    display: flex;
    flex-direction: column;
    padding: 8px;
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
  }
</style>
