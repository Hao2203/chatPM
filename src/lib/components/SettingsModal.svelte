<script lang="ts">
  let {
    show = false,
    apiKey = "",
    loading = false,
    onapikeychange = (_val: string) => {},
    onclose = () => {},
    onsave = () => {},
    onclear = () => {},
  }: {
    show: boolean;
    apiKey: string;
    loading: boolean;
    onapikeychange: (val: string) => void;
    onclose: () => void;
    onsave: () => void;
    onclear: () => void;
  } = $props();

  type View = "menu" | "apikey" | "clear-data";
  let view = $state<View>("menu");
  let clearConfirm = $state(false);

  // Reset to menu when opening
  $effect(() => {
    if (show) view = "menu";
    clearConfirm = false;
  });
</script>

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="settings-overlay"
    role="presentation"
    onclick={onclose}
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
        <button class="btn-close" onclick={onclose} title="关闭">
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
            oninput={(e) => onapikeychange((e.target as HTMLInputElement).value)}
            placeholder="sk-..."
            disabled={loading}
          />
        </label>
        <div class="settings-actions">
          <button class="btn-primary" onclick={onsave} disabled={loading}>
            {loading ? "配置中..." : "保存"}
          </button>
          <button class="btn-cancel" onclick={() => (view = "menu")}>取消</button>
        </div>

      {:else if view === "clear-data"}
        {#if clearConfirm}
          <p class="confirm-message">确定要清除所有数据吗？这将删除所有聊天记录、会话标题和 API Key 配置，重置应用到初始状态。此操作无法撤销。</p>
          <div class="settings-actions">
            <button class="btn-danger" onclick={onclear}>确认清除</button>
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
