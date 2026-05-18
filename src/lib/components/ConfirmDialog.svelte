<script lang="ts">
  let {
    show = false,
    title = "确认",
    message = "",
    confirmText = "确认删除",
    cancelText = "取消",
    danger = false,
    onconfirm = () => {},
    oncancel = () => {},
  }: {
    show: boolean;
    title: string;
    message: string;
    confirmText: string;
    cancelText: string;
    danger: boolean;
    onconfirm: () => void;
    oncancel: () => void;
  } = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      oncancel();
    } else if (e.key === "Enter") {
      onconfirm();
    }
  }
</script>

<svelte:window onkeydown={show ? handleKeydown : undefined} />

{#if show}
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <div
    class="confirm-overlay"
    role="presentation"
    onclick={oncancel}
  >
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <div
      class="confirm-panel"
      role="dialog"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <div class="confirm-header">
        <h2>{title}</h2>
        <button
          class="btn-close"
          onclick={oncancel}
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
      <p class="confirm-message">{message}</p>
      <div class="confirm-actions">
        <button
          class="btn-confirm"
          class:danger
          onclick={onconfirm}
        >
          {confirmText}
        </button>
        <button class="btn-cancel" onclick={oncancel}>
          {cancelText}
        </button>
      </div>
    </div>
  </div>
{/if}

<style>
  .confirm-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.6);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 110;
  }

  .confirm-panel {
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    padding: 24px;
    width: 400px;
    max-width: 90vw;
  }

  .confirm-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    margin-bottom: 16px;
  }

  .confirm-header h2 {
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

  .confirm-message {
    color: var(--text-secondary);
    font-size: 14px;
    line-height: 1.5;
    margin-bottom: 20px;
  }

  .confirm-actions {
    display: flex;
    gap: 10px;
    justify-content: flex-end;
  }

  .btn-confirm {
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

  .btn-confirm:hover {
    background: var(--accent-hover);
  }

  .btn-confirm.danger {
    background: var(--danger);
  }

  .btn-confirm.danger:hover {
    background: #dc2626;
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

  @media (max-width: 768px) {
    .confirm-panel {
      width: 100%;
      max-width: calc(100vw - 32px);
      padding: 20px;
    }

    .confirm-header h2 {
      font-size: 16px;
    }
  }
</style>
