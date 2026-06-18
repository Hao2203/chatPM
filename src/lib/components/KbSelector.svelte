<script lang="ts">
  let {
    activeKbs = [] as { kb_id: string; name: string }[],
    allKbs = [] as { kb_id: string; name: string }[],
    onToggle = (_kbId: string) => {},
  } = $props();

  let showDropdown = $state(false);
  let dropdownEl: HTMLDivElement;

  // 点击外部关闭下拉
  function handleClickOutside(e: MouseEvent) {
    if (dropdownEl && !dropdownEl.contains(e.target as Node)) {
      showDropdown = false;
    }
  }

  $effect(() => {
    if (showDropdown) {
      document.addEventListener("click", handleClickOutside);
    }
    return () => {
      document.removeEventListener("click", handleClickOutside);
    };
  });

  const availableKbs = $derived(allKbs.filter(k => !activeKbs.find(a => a.kb_id === k.kb_id)));
</script>

<div class="kb-selector">
  {#each activeKbs as kb (kb.kb_id)}
    <button class="kb-chip active" onclick={() => onToggle(kb.kb_id)} title="点击取消引用">
      <span class="chip-icon">@</span>
      {kb.name}
      <span class="chip-remove">&times;</span>
    </button>
  {/each}

  <div class="kb-dropdown-wrapper" bind:this={dropdownEl}>
    <button class="kb-add-btn" onclick={() => showDropdown = !showDropdown}>
      + 资料库
    </button>

    {#if showDropdown && availableKbs.length > 0}
      <div class="kb-dropdown">
        {#each availableKbs as kb (kb.kb_id)}
          <button
            class="kb-dropdown-item"
            onclick={() => {
              onToggle(kb.kb_id);
              showDropdown = false;
            }}
          >
            @{kb.name}
          </button>
        {/each}
      </div>
    {:else if showDropdown}
      <div class="kb-dropdown empty">暂无可用资料库</div>
    {/if}
  </div>
</div>

<style>
  .kb-selector {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    align-items: center;
    padding: 4px 0 8px;
  }

  .kb-chip {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 3px 10px;
    border: 1px solid var(--accent);
    border-radius: 20px;
    background: var(--accent-light, rgba(99, 102, 241, 0.1));
    color: var(--accent);
    font-size: 12px;
    font-family: var(--font-sans);
    cursor: pointer;
    transition: background 0.15s;
    white-space: nowrap;
  }

  .kb-chip:hover {
    background: var(--accent-light-hover, rgba(99, 102, 241, 0.2));
  }

  .chip-icon {
    font-weight: bold;
    font-size: 13px;
  }

  .chip-remove {
    font-size: 14px;
    margin-left: 2px;
    opacity: 0.6;
  }

  .chip-remove:hover {
    opacity: 1;
  }

  .kb-add-btn {
    display: inline-flex;
    align-items: center;
    padding: 3px 10px;
    border: 1px dashed var(--border-color);
    border-radius: 20px;
    background: transparent;
    color: var(--text-muted);
    font-size: 12px;
    font-family: var(--font-sans);
    cursor: pointer;
    transition: all 0.15s;
    white-space: nowrap;
  }

  .kb-add-btn:hover {
    border-color: var(--accent);
    color: var(--accent);
  }

  .kb-dropdown-wrapper {
    position: relative;
  }

  .kb-dropdown {
    position: absolute;
    top: 100%;
    left: 0;
    margin-top: 4px;
    background: var(--bg-sidebar);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-md);
    box-shadow: 0 4px 12px rgba(0, 0, 0, 0.2);
    min-width: 180px;
    max-height: 240px;
    overflow-y: auto;
    z-index: 100;
    padding: 4px;
  }

  .kb-dropdown.empty {
    padding: 12px;
    text-align: center;
    color: var(--text-muted);
    font-size: 13px;
  }

  .kb-dropdown-item {
    display: block;
    width: 100%;
    text-align: left;
    padding: 8px 12px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
    cursor: pointer;
    transition: background 0.1s;
  }

  .kb-dropdown-item:hover {
    background: var(--bg-hover);
  }
</style>
