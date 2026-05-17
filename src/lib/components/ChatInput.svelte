<script lang="ts">
  let {
    inputText = "",
    sending = false,
    oninputtextchange = (_val: string) => {},
    onsend = () => {},
  }: {
    inputText: string;
    sending: boolean;
    oninputtextchange: (val: string) => void;
    onsend: () => void;
  } = $props();

  function handleKeydown(e: KeyboardEvent) {
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      onsend();
    }
  }

  function autoResize(e: Event) {
    const ta = e.target as HTMLTextAreaElement;
    ta.style.height = "auto";
    ta.style.height = Math.min(ta.scrollHeight, 200) + "px";
  }

  function onInput(e: Event) {
    autoResize(e);
    oninputtextchange((e.target as HTMLTextAreaElement).value);
  }
</script>

<div class="input-bar">
  <div class="input-wrapper">
    <textarea
      value={inputText}
      onkeydown={handleKeydown}
      oninput={onInput}
      placeholder="发送消息..."
      rows="1"
      disabled={sending}
    ></textarea>
    <button
      class="btn-send"
      onclick={onsend}
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

<style>
  .input-bar {
    padding: 12px 40px 20px;
    max-width: 820px;
    width: 100%;
    margin: 0 auto;
    box-sizing: border-box;
  }

  /* Narrower desktops: reduce horizontal padding before mobile breakpoint */
  @media (max-width: 900px) {
    .input-bar {
      padding: 12px 20px 20px;
    }
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

  @media (max-width: 768px) {
    .input-bar {
      padding: 8px 12px 14px;
      max-width: 100%;
    }

    .input-wrapper {
      padding: 6px 6px 6px 12px;
    }

    .input-wrapper textarea {
      font-size: 14px;
    }

    .input-hint {
      display: none;
    }
  }

  @media (max-width: 480px) {
    .input-bar {
      padding: 6px 8px 12px;
    }
  }
</style>
