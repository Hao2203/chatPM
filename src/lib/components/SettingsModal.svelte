<script lang="ts">
  let {
    show = false,
    apiKey = "",
    model = "deepseek-v4-flash",
    loading = false,
    syncActive = false,
    syncTicket = "",
    onApiKeyChange = (_val: string) => {},
    onModelChange = (_val: string) => {},
    onClose = () => {},
    onSave = () => {},
    onClear = () => {},
    onCreateSync = () => {},
    onJoinSync = (_ticket: string) => {},
    onStopSync = () => {},
  }: {
    show: boolean;
    apiKey: string;
    model: string;
    loading: boolean;
    syncActive: boolean;
    syncTicket: string;
    onApiKeyChange: (val: string) => void;
    onModelChange: (val: string) => void;
    onClose: () => void;
    onSave: () => void;
    onClear: () => void;
    onCreateSync: () => void;
    onJoinSync: (ticket: string) => void;
    onStopSync: () => void;
  } = $props();

  type View = "menu" | "apikey" | "model" | "clear-data" | "sync";
  let view = $state<View>("menu");
  let clearConfirm = $state(false);
  let syncLoading = $state(false);
  let joinTicket = $state("");

  // Reset to menu when opening
  $effect(() => {
    if (show) view = "menu";
    clearConfirm = false;
  });

  const MODEL_OPTIONS = [
    { value: "deepseek-v4-flash", label: "DeepSeek V4 Flash" },
    { value: "deepseek-v4-pro", label: "DeepSeek V4 Pro" },
  ];
</script>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="settings-overlay"
    role="presentation"
    onclick={onClose}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="settings-panel"
      role="dialog"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="settings-header">
        {#if view !== "menu"}
          <button class="btn-back" onclick={() => (view = "menu")} title="返回">
            <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <polyline points="15 18 9 12 15 6"></polyline>
            </svg>
          </button>
        {/if}
        <h2>设置</h2>
        <button class="btn-close" onclick={onClose} title="关闭">
          <svg width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
            <line x1="18" y1="6" x2="6" y2="18"></line>
            <line x1="6" y1="6" x2="18" y2="18"></line>
          </svg>
        </button>
      </div>

      {#if view === "menu"}
        <div class="menu-list">
          <button class="menu-item" onclick={() => (view = "apikey")}>
            <div class="menu-item-text">
              <span class="menu-item-title">API Key</span>
              <span class="menu-item-desc">配置 DeepSeek API Key</span>
            </div>
            <svg class="menu-item-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <polyline points="9 18 15 12 9 6"></polyline>
            </svg>
          </button>
          <button class="menu-item" onclick={() => (view = "model")}>
            <div class="menu-item-text">
              <span class="menu-item-title">模型选择</span>
              <span class="menu-item-desc">当前：{MODEL_OPTIONS.find(o => o.value === model)?.label ?? model}</span>
            </div>
            <svg class="menu-item-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <polyline points="9 18 15 12 9 6"></polyline>
            </svg>
          </button>
          <button class="menu-item" onclick={() => (view = "sync")}>
            <div class="menu-item-text">
              <span class="menu-item-title">设备同步</span>
              <span class="menu-item-desc">{syncActive ? "同步中..." : "P2P 跨设备同步"}</span>
            </div>
            <svg class="menu-item-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <polyline points="9 18 15 12 9 6"></polyline>
            </svg>
          </button>
          <button class="menu-item" onclick={() => (view = "clear-data")}>
            <div class="menu-item-text">
              <span class="menu-item-title">清除所有数据</span>
              <span class="menu-item-desc">删除所有聊天记录和配置数据</span>
            </div>
            <svg class="menu-item-chevron" width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round">
              <polyline points="9 18 15 12 9 6"></polyline>
            </svg>
          </button>
        </div>

      {:else if view === "apikey"}
        <label class="setting-label">
          <span>DeepSeek API Key</span>
          <input
            type="password"
            value={apiKey}
            oninput={(e) => onApiKeyChange((e.target as HTMLInputElement).value)}
            placeholder="sk-..."
            disabled={loading}
          />
        </label>
        <div class="settings-actions">
          <button class="btn-primary" onclick={onSave} disabled={loading}>
            {loading ? "配置中..." : "保存"}
          </button>
          <button class="btn-cancel" onclick={() => (view = "menu")}>取消</button>
        </div>

      {:else if view === "model"}
        <div class="model-select-label">
          <span>选择模型</span>
          <select
            value={model}
            onchange={(e) => onModelChange((e.target as HTMLSelectElement).value)}
            disabled={loading}
          >
            {#each MODEL_OPTIONS as opt}
              <option value={opt.value}>{opt.label}</option>
            {/each}
          </select>
        </div>
        <div class="settings-actions">
          <button class="btn-cancel" onclick={() => (view = "menu")}>返回</button>
        </div>

      {:else if view === "sync"}
        <div class="sync-panel">
          {#if syncActive}
            <div class="sync-status">
              <span class="sync-indicator active"></span>
              <span>同步已启用</span>
            </div>

            {#if syncTicket}
              <label class="setting-label" style="margin-bottom:12px">
                <span>同步凭证（ticket）</span>
                <input type="text" value={syncTicket} readonly class="ticket-display" />
              </label>
            {/if}

            <button class="btn-danger" onclick={() => { onStopSync(); }} disabled={syncLoading}>
              {syncLoading ? "停止中..." : "停止同步"}
            </button>
          {:else}
            <div class="sync-status">
              <span class="sync-indicator"></span>
              <span>同步未启用</span>
            </div>

            <p class="sync-info">
              通过 P2P 网络与其他设备同步聊天记录。作为发起者创建同步链获取凭证，
              其他设备凭凭证加入同一同步链。
            </p>

            <button
              class="btn-primary"
              style="width:100%"
              onclick={async () => {
                syncLoading = true;
                try {
                  await onCreateSync();
                  view = "sync";
                } finally {
                  syncLoading = false;
                }
              }}
              disabled={syncLoading}
            >
              {syncLoading ? "创建中..." : "创建同步链"}
            </button>

            <div class="sync-divider">
              <span>或凭凭证加入</span>
            </div>

            <label class="setting-label" style="margin-bottom:12px">
              <span>同步凭证（ticket）</span>
              <input
                type="text"
                bind:value={joinTicket}
                placeholder="粘贴同步凭证..."
                disabled={syncLoading}
              />
            </label>

            <button
              class="btn-primary"
              style="width:100%"
              onclick={async () => {
                if (!joinTicket.trim()) return;
                syncLoading = true;
                try {
                  await onJoinSync(joinTicket.trim());
                  joinTicket = "";
                  view = "sync";
                } finally {
                  syncLoading = false;
                }
              }}
              disabled={syncLoading || !joinTicket.trim()}
            >
              {syncLoading ? "加入中..." : "加入同步链"}
            </button>
          {/if}
        </div>
        <div class="settings-actions" style="margin-top:16px">
          <button class="btn-cancel" onclick={() => (view = "menu")}>返回</button>
        </div>

      {:else if view === "clear-data"}
        {#if clearConfirm}
          <p class="confirm-message">确定要清除所有数据吗？这将删除所有聊天记录、会话标题和 API Key 配置，重置应用到初始状态。此操作无法撤销。</p>
          <div class="settings-actions">
            <button class="btn-danger" onclick={onClear}>确认清除</button>
            <button class="btn-cancel" onclick={() => (clearConfirm = false)}>取消</button>
          </div>
        {:else}
          <p class="confirm-message">此操作将删除所有聊天记录、会话标题和 API Key 配置，重置应用到初始状态。此操作无法撤销。</p>
          <div class="settings-actions">
            <button class="btn-danger" onclick={() => (clearConfirm = true)}>清除所有数据</button>
            <button class="btn-cancel" onclick={() => (view = "menu")}>取消</button>
          </div>
        {/if}
      {/if}
    </div>
  </div>
{/if}

<style>
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
    gap: 4px;
    margin-bottom: 20px;
  }

  .settings-header h2 {
    flex: 1;
    font-size: 18px;
    font-weight: 600;
  }

  .btn-back {
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
    transition: color 0.15s, background 0.15s;
    flex-shrink: 0;
  }

  .btn-back:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
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
    transition: color 0.15s, background 0.15s;
    flex-shrink: 0;
  }

  .btn-close:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
  }

  /* ── Menu ────────────────────────────────────────── */

  .menu-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }

  .menu-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    width: 100%;
    padding: 14px 16px;
    border: none;
    border-radius: var(--radius);
    background: transparent;
    cursor: pointer;
    transition: background 0.15s;
    text-align: left;
  }

  .menu-item:hover {
    background: var(--bg-hover);
  }

  .menu-item-text {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .menu-item-title {
    font-size: 14px;
    font-weight: 500;
    color: var(--text-primary);
  }

  .menu-item-desc {
    font-size: 12px;
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .menu-item-chevron {
    flex-shrink: 0;
    color: var(--text-muted);
  }

  /* ── API Key form ────────────────────────────────── */

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

  .model-select-label {
    display: flex;
    flex-direction: column;
    gap: 8px;
    font-size: 14px;
    color: var(--text-secondary);
    margin-bottom: 20px;
  }

  .model-select-label select {
    padding: 10px 36px 10px 14px;
    border-radius: var(--radius);
    border: 1px solid var(--border-color);
    background: var(--bg-primary);
    color: var(--text-primary);
    font-size: 14px;
    outline: none;
    cursor: pointer;
    transition: border-color 0.15s;
    appearance: none;
    background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%238e8ea0' stroke-width='2.5' stroke-linecap='round'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E");
    background-repeat: no-repeat;
    background-position: right 12px center;
    background-size: 12px;
  }

  .model-select-label select:focus {
    border-color: var(--accent);
  }

  .model-select-label select:disabled {
    opacity: 0.5;
    cursor: not-allowed;
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
    transition: background 0.15s, opacity 0.15s;
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

  .btn-danger {
    padding: 10px 20px;
    border: 1px solid var(--danger);
    border-radius: var(--radius);
    background: transparent;
    color: var(--danger);
    font-size: 14px;
    cursor: pointer;
    transition: background 0.15s;
  }

  .btn-danger:hover {
    background: rgba(239, 68, 68, 0.1);
  }

  /* ── Confirmation message ────────────────────────── */

  .confirm-message {
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.5;
    margin: 0 0 20px;
  }

  /* ── Sync Panel ───────────────────────────────────── */

  .sync-panel {
    display: flex;
    flex-direction: column;
    gap: 12px;
  }

  .sync-status {
    display: flex;
    align-items: center;
    gap: 8px;
    font-size: 14px;
    color: var(--text-primary);
    font-weight: 500;
    margin-bottom: 4px;
  }

  .sync-indicator {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--text-muted);
    flex-shrink: 0;
  }

  .sync-indicator.active {
    background: var(--accent);
    box-shadow: 0 0 6px var(--accent);
  }

  .sync-info {
    color: var(--text-muted);
    font-size: 13px;
    line-height: 1.5;
    margin: 0;
  }

  .sync-divider {
    display: flex;
    align-items: center;
    gap: 12px;
    color: var(--text-muted);
    font-size: 12px;
  }

  .sync-divider::before,
  .sync-divider::after {
    content: "";
    flex: 1;
    height: 1px;
    background: var(--border-color);
  }

  .ticket-display {
    cursor: text;
    user-select: all;
    font-family: monospace;
    font-size: 12px;
    word-break: break-all;
  }

  @media (max-width: 768px) {
    .settings-panel {
      width: 100%;
      max-width: calc(100vw - 32px);
      padding: 20px;
      border-radius: var(--radius-lg);
    }

    .settings-header h2 {
      font-size: 16px;
    }
  }
</style>
