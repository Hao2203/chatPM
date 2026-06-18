<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";

  let {
    show = false,
    onClose,
  }: {
    show: boolean;
    onClose: () => void;
  } = $props();

  interface KbInfo {
    kb_id: string;
    name: string;
    created_at: string;
    document_count: number;
    total_chunks: number;
  }

  interface KbDocInfo {
    doc_id: string;
    kb_id: string;
    title: string;
    chunk_count: number;
    char_count: number;
    created_at: string;
  }

  let knowledgeBases = $state<KbInfo[]>([]);
  let selectedKbId = $state<string | null>(null);
  let documents = $state<KbDocInfo[]>([]);
  let newKbName = $state("");
  let newDocTitle = $state("");
  let newDocText = $state("");
  let loading = $state(false);
  let error = $state("");

  // 搜索
  let searchQuery = $state("");
  let searchResults = $state<{ chunk_id: string; document_id: string; chunk_index: number; content: string; score: number }[]>([]);

  async function loadKbs() {
    try {
      knowledgeBases = await invoke<KbInfo[]>("list_knowledge_bases");
    } catch (e: any) {
      error = getErrorMessage(e);
    }
  }

  async function selectKb(kbId: string) {
    selectedKbId = kbId;
    documents = [];
    searchResults = [];
    searchQuery = "";
    try {
      documents = await invoke<KbDocInfo[]>("list_kb_documents", { kbId });
    } catch (e: any) {
      error = getErrorMessage(e);
    }
  }

  $effect(() => {
    if (show) {
      loadKbs();
      selectedKbId = null;
      documents = [];
      searchResults = [];
      error = "";
    }
  });

  async function createKb() {
    if (!newKbName.trim()) return;
    loading = true;
    error = "";
    try {
      await invoke("create_knowledge_base", { name: newKbName.trim() });
      newKbName = "";
      await loadKbs();
    } catch (e: any) {
      error = getErrorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function deleteKb(kbId: string) {
    loading = true;
    error = "";
    try {
      await invoke("delete_knowledge_base", { kbId });
      if (selectedKbId === kbId) selectedKbId = null;
      await loadKbs();
    } catch (e: any) {
      error = getErrorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function addDocument() {
    if (!newDocTitle.trim() || !newDocText.trim() || !selectedKbId) return;
    loading = true;
    error = "";
    try {
      await invoke("add_kb_document", {
        kbId: selectedKbId,
        title: newDocTitle.trim(),
        text: newDocText,
      });
      newDocTitle = "";
      newDocText = "";
      await selectKb(selectedKbId);
    } catch (e: any) {
      error = getErrorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function deleteDocument(docId: string) {
    if (!selectedKbId) return;
    loading = true;
    error = "";
    try {
      await invoke("delete_kb_document", { kbId: selectedKbId, docId });
      await selectKb(selectedKbId);
    } catch (e: any) {
      error = getErrorMessage(e);
    } finally {
      loading = false;
    }
  }

  async function searchKb() {
    if (!searchQuery.trim() || !selectedKbId) return;
    loading = true;
    error = "";
    try {
      searchResults = await invoke("search_knowledge_base", {
        kbId: selectedKbId,
        query: searchQuery.trim(),
        limit: 10,
      });
    } catch (e: any) {
      error = getErrorMessage(e);
    } finally {
      loading = false;
    }
  }

  function getErrorMessage(e: any): string {
    if (typeof e === "string") return e;
    if (e?.message) return e.message;
    return String(e);
  }
</script>

{#if show}
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="modal-overlay" role="dialog">
    <div class="modal-content" role="document">
      <div class="modal-header">
        <h2>资料库管理</h2>
        <button class="btn-close" onclick={onClose}>&times;</button>
      </div>

      {#if error}
        <div class="error-msg">{error}</div>
      {/if}

      <div class="modal-body">
        <!-- 左侧：资料库列表 -->
        <div class="kb-list-panel">
          <h3>资料库</h3>

          <div class="new-kb-form">
            <input
              type="text"
              bind:value={newKbName}
              placeholder="新建资料库名称..."
              onkeydown={(e) => e.key === "Enter" && createKb()}
            />
            <button onclick={createKb} disabled={!newKbName.trim() || loading}>创建</button>
          </div>

          <div class="kb-list">
            {#each knowledgeBases as kb (kb.kb_id)}
              <button
                class="kb-item"
                class:active={selectedKbId === kb.kb_id}
                onclick={() => selectKb(kb.kb_id)}
              >
                <span class="kb-name">@{kb.name}</span>
                <span class="kb-stats">{kb.document_count} 文档 · {kb.total_chunks} 块</span>
              </button>
            {/each}
            {#if knowledgeBases.length === 0}
              <p class="empty-text">暂无资料库，请创建第一个</p>
            {/if}
          </div>
        </div>

        <!-- 右侧：文档管理 -->
        <div class="kb-detail-panel">
          {#if selectedKbId}
            <h3>
              @{knowledgeBases.find(k => k.kb_id === selectedKbId)?.name ?? ""}
              <button
                class="btn-delete-sm"
                onclick={() => deleteKb(selectedKbId!)}
                disabled={loading}
              >删除资料库</button>
            </h3>

            <!-- 搜索 -->
            <div class="search-form">
              <input
                type="text"
                bind:value={searchQuery}
                placeholder="搜索知识库..."
                onkeydown={(e) => e.key === "Enter" && searchKb()}
              />
              <button onclick={searchKb} disabled={!searchQuery.trim() || loading}>搜索</button>
            </div>

            {#if searchResults.length > 0}
              <h4>搜索结果</h4>
              <div class="search-results">
                {#each searchResults as r, i}
                  <div class="search-item">
                    <div class="search-score">相关度: {r.score.toFixed(3)}</div>
                    <div class="search-content">{r.content.slice(0, 200)}...</div>
                  </div>
                {/each}
              </div>
            {/if}

            <!-- 添加文档 -->
            <h4>添加文档</h4>
            <div class="add-doc-form">
              <input
                type="text"
                bind:value={newDocTitle}
                placeholder="文档标题..."
              />
              <textarea
                bind:value={newDocText}
                placeholder="文档内容..."
                rows={6}
              ></textarea>
              <button
                onclick={addDocument}
                disabled={!newDocTitle.trim() || !newDocText.trim() || loading}
              >添加文档</button>
            </div>

            <!-- 文档列表 -->
            <h4>文档列表 ({documents.length})</h4>
            <div class="doc-list">
              {#each documents as doc (doc.doc_id)}
                <div class="doc-item">
                  <div class="doc-info">
                    <span class="doc-title">{doc.title}</span>
                    <span class="doc-stats">{doc.chunk_count} 块 · {doc.char_count} 字符</span>
                  </div>
                  <button
                    class="btn-delete-xs"
                    onclick={() => deleteDocument(doc.doc_id)}
                    disabled={loading}
                  >删除</button>
                </div>
              {/each}
              {#if documents.length === 0}
                <p class="empty-text">暂无文档</p>
              {/if}
            </div>
          {:else}
            <p class="empty-text">选择左侧资料库查看详情</p>
          {/if}
        </div>
      </div>
    </div>
  </div>
{/if}

<style>
  .modal-overlay {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.5);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
  }

  .modal-content {
    background: var(--bg-sidebar);
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    width: 90vw;
    max-width: 900px;
    max-height: 85vh;
    display: flex;
    flex-direction: column;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3);
  }

  .modal-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 20px;
    border-bottom: 1px solid var(--border-color);
  }

  .modal-header h2 {
    margin: 0;
    font-size: 18px;
    color: var(--text-primary);
  }

  .btn-close {
    background: none;
    border: none;
    font-size: 24px;
    color: var(--text-muted);
    cursor: pointer;
    padding: 0 4px;
  }

  .btn-close:hover {
    color: var(--text-primary);
  }

  .error-msg {
    background: rgba(239, 68, 68, 0.1);
    color: #ef4444;
    padding: 8px 16px;
    font-size: 13px;
    border-bottom: 1px solid var(--border-color);
  }

  .modal-body {
    display: flex;
    flex: 1;
    overflow: hidden;
  }

  .kb-list-panel {
    width: 260px;
    flex-shrink: 0;
    padding: 16px;
    border-right: 1px solid var(--border-color);
    overflow-y: auto;
    display: flex;
    flex-direction: column;
    gap: 10px;
  }

  .kb-detail-panel {
    flex: 1;
    padding: 16px 20px;
    overflow-y: auto;
  }

  h3 {
    margin: 0 0 12px;
    font-size: 15px;
    color: var(--text-primary);
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
  }

  h4 {
    margin: 16px 0 8px;
    font-size: 13px;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .new-kb-form, .search-form, .add-doc-form {
    display: flex;
    gap: 6px;
  }

  .new-kb-form input, .search-form input, .add-doc-form input {
    flex: 1;
    padding: 6px 10px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
  }

  .add-doc-form {
    flex-direction: column;
  }

  .add-doc-form textarea {
    width: 100%;
    padding: 8px 10px;
    border: 1px solid var(--border-color);
    border-radius: 6px;
    background: var(--bg-input);
    color: var(--text-primary);
    font-size: 13px;
    font-family: var(--font-sans);
    resize: vertical;
    box-sizing: border-box;
  }

  button {
    padding: 6px 14px;
    border: none;
    border-radius: 6px;
    background: var(--accent);
    color: #fff;
    font-size: 13px;
    font-family: var(--font-sans);
    cursor: pointer;
    white-space: nowrap;
  }

  button:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .kb-list {
    display: flex;
    flex-direction: column;
    gap: 4px;
    flex: 1;
    overflow-y: auto;
  }

  .kb-item {
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    width: 100%;
    padding: 10px 12px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text-primary);
    text-align: left;
    cursor: pointer;
    transition: background 0.1s;
    gap: 2px;
  }

  .kb-item:hover {
    background: var(--bg-hover);
  }

  .kb-item.active {
    background: var(--accent-light, rgba(99, 102, 241, 0.15));
  }

  .kb-name {
    font-size: 14px;
    font-weight: 500;
  }

  .kb-stats, .doc-stats {
    font-size: 11px;
    color: var(--text-muted);
  }

  .empty-text {
    color: var(--text-muted);
    font-size: 13px;
    text-align: center;
    padding: 20px;
  }

  .btn-delete-sm {
    background: rgba(239, 68, 68, 0.15);
    color: #ef4444;
    font-size: 11px;
    padding: 4px 10px;
  }

  .btn-delete-xs {
    background: transparent;
    color: var(--text-muted);
    font-size: 11px;
    padding: 3px 8px;
    border: 1px solid var(--border-color);
  }

  .btn-delete-xs:hover {
    color: #ef4444;
    border-color: #ef4444;
  }

  .doc-list {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .doc-item {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 10px 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    gap: 8px;
  }

  .doc-info {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .doc-title {
    font-size: 13px;
    color: var(--text-primary);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .search-results {
    display: flex;
    flex-direction: column;
    gap: 8px;
    margin-bottom: 8px;
  }

  .search-item {
    padding: 10px 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
  }

  .search-score {
    font-size: 11px;
    color: var(--accent);
    margin-bottom: 4px;
  }

  .search-content {
    font-size: 12px;
    color: var(--text-primary);
    line-height: 1.5;
  }
</style>
