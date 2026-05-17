<script lang="ts">
  import { marked } from "marked";

  // ── Marked configuration ───────────────────────────────────
  marked.use({
    gfm: true,
    breaks: true,
  });

  function renderMarkdown(text: string): string {
    if (!text) return "";
    return marked.parse(text) as string;
  }

  interface Message {
    id: number;
    role: "user" | "assistant";
    content: string;
    streaming: boolean;
  }

  let { messages = [] }: { messages: Message[] } = $props();
</script>

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
        <div class="message-bubble markdown-body">
          {@html renderMarkdown(msg.content)}
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

<style>
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

  @media (max-width: 768px) {
    .message-row {
      padding: 12px 16px 12px 52px;
      gap: 10px;
      max-width: 100%;
    }

    .message-avatar {
      width: 26px;
    }

    .avatar {
      width: 26px;
      height: 26px;
    }

    .message-bubble {
      font-size: 14px;
    }
  }

  @media (max-width: 480px) {
    .message-row {
      padding: 10px 12px 10px 44px;
    }
  }
</style>
