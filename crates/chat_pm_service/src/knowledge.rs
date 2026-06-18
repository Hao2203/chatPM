use std::path::PathBuf;
use std::sync::{Arc, Mutex as StdMutex};

use chat_pm_database::{ChatDb, DocRecord, KbRecord};
use chat_pm_knowledge::{
    Bm25Index, ChunkConfig, DocumentChunk, EdgeVectorStore, Embed, HybridSearcher,
    InMemoryBm25Index, KnowledgeBase, KnowledgeBaseId, KnowledgeBaseName, KnowledgeError,
    SearchResult, chunk_text,
};
use chat_pm_session::session::SessionId;
use tokio::sync::Mutex as TokioMutex;
use tracing::{debug, info, warn};

use crate::session::CommandError;

/// 知识库服务，编排嵌入、向量存储、BM25 索引和数据库元数据。
pub struct KnowledgeService {
    db: ChatDb,
    embedder: Arc<dyn Embed>,
    /// EdgeShard 存储的根目录。
    stores_dir: PathBuf,
    /// 已打开的 BM25 索引：kb_id -> InMemoryBm25Index。
    bm25_indexes: TokioMutex<std::collections::HashMap<KnowledgeBaseId, InMemoryBm25Index>>,
    /// 已打开的向量存储：kb_id -> EdgeVectorStore。
    open_stores: TokioMutex<std::collections::HashMap<KnowledgeBaseId, Arc<StdMutex<EdgeVectorStore>>>>,
    /// 混合检索器。
    searcher: HybridSearcher,
}

impl KnowledgeService {
    /// 创建新的知识库服务。
    pub fn new(
        db: ChatDb,
        embedder: Arc<dyn Embed>,
        stores_dir: PathBuf,
    ) -> Self {
        Self {
            db,
            embedder,
            stores_dir,
            bm25_indexes: TokioMutex::new(std::collections::HashMap::new()),
            open_stores: TokioMutex::new(std::collections::HashMap::new()),
            searcher: HybridSearcher::default(),
        }
    }

    // ── KB CRUD ─────────────────────────────────────────────────

    /// 创建新的知识库。
    pub async fn create_kb(&self, name: &KnowledgeBaseName) -> Result<KnowledgeBase, CommandError> {
        if !name.is_valid() {
            return Err(CommandError::Knowledge(KnowledgeError::KnowledgeBaseAlreadyExists(
                "名称无效".to_string(),
            )));
        }

        let kb_id = KnowledgeBaseId::new();

        // 创建 SQLite 元数据
        self.db.create_knowledge_base(kb_id, name.as_str())?;

        // 创建 EdgeShard 目录
        let shard_dir = self.kb_dir(kb_id);
        let dimension = self.embedder.dimension();
        let store = EdgeVectorStore::new(&shard_dir, dimension)
            .map_err(|e| CommandError::Knowledge(e))?;

        // 创建 BM25 索引
        let bm25_index = InMemoryBm25Index::new();
        let mut bm25_map = self.bm25_indexes.lock().await;
        bm25_map.insert(kb_id, bm25_index);

        // 缓存向量存储
        let mut stores = self.open_stores.lock().await;
        stores.insert(kb_id, Arc::new(StdMutex::new(store)));

        info!(%kb_id, name = name.as_str(), "知识库已创建");

        Ok(KnowledgeBase {
            id: kb_id,
            name: name.clone(),
            document_count: 0,
            total_chunks: 0,
        })
    }

    /// 列出所有知识库。
    pub async fn list_kbs(&self) -> Result<Vec<KbRecord>, CommandError> {
        Ok(self.db.list_knowledge_bases()?)
    }

    /// 重命名知识库。
    pub async fn rename_kb(
        &self,
        kb_id: KnowledgeBaseId,
        new_name: &KnowledgeBaseName,
    ) -> Result<(), CommandError> {
        if !new_name.is_valid() {
            return Err(CommandError::Knowledge(KnowledgeError::KnowledgeBaseAlreadyExists(
                "名称无效".to_string(),
            )));
        }
        Ok(self.db.rename_knowledge_base(kb_id, new_name.as_str())?)
    }

    /// 删除知识库（级联删除文档和向量数据）。
    pub async fn delete_kb(&self, kb_id: KnowledgeBaseId) -> Result<(), CommandError> {
        // 删除 SQLite 元数据
        self.db.delete_knowledge_base(kb_id)?;

        // 清理缓存
        let mut bm25_map = self.bm25_indexes.lock().await;
        bm25_map.remove(&kb_id);

        let mut stores = self.open_stores.lock().await;
        stores.remove(&kb_id);

        // 删除磁盘目录
        let shard_dir = self.kb_dir(kb_id);
        if shard_dir.exists() {
            tokio::task::spawn_blocking(move || {
                std::fs::remove_dir_all(&shard_dir)
            })
            .await
            .map_err(|e| CommandError::Internal(e.into()))?
            .ok();
        }

        info!(%kb_id, "知识库已删除");
        Ok(())
    }

    // ── Document Management ─────────────────────────────────────

    /// 向知识库添加文档。
    pub async fn add_document(
        &self,
        kb_id: KnowledgeBaseId,
        title: String,
        text: &str,
    ) -> Result<DocRecord, CommandError> {
        let config = ChunkConfig::default();
        let chunk_texts = chunk_text(text, &config);

        if chunk_texts.is_empty() {
            return Err(CommandError::Knowledge(KnowledgeError::DocumentTooShort(
                config.min_chunk_size,
            )));
        }

        let doc_id = uuid::Uuid::now_v7().to_string();

        // 构建 DocumentChunk 列表
        let chunks: Vec<DocumentChunk> = chunk_texts
            .iter()
            .enumerate()
            .map(|(i, content)| DocumentChunk {
                chunk_id: chat_pm_knowledge::ChunkId::new(),
                knowledge_base_id: kb_id.to_string(),
                document_id: doc_id.clone(),
                chunk_index: i,
                content: content.clone(),
                char_count: content.len(),
            })
            .collect();

        let chunk_count = chunks.len();
        let total_chars: usize = chunks.iter().map(|c| c.char_count).sum();

        // 生成嵌入向量
        let vectors = {
            let embedder = Arc::clone(&self.embedder);
            let texts = chunks.iter().map(|c| c.content.clone()).collect::<Vec<_>>();
            tokio::task::spawn_blocking(move || {
                let refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
                embedder.embed_batch(&refs)
            })
            .await
            .map_err(|e| CommandError::Internal(e.into()))?
            .map_err(|e| CommandError::Knowledge(e))?
        };

        // 写入向量存储（同步操作，使用 spawn_blocking）
        {
            let stores = self.open_stores.lock().await;
            let store = self.get_or_open_store_inner(kb_id, &stores)?;
            let store = Arc::clone(&store);
            let chunks_clone = chunks.clone();
            let vectors_clone = vectors.clone();
            tokio::task::spawn_blocking(move || {
                let s = store.lock().map_err(|e| {
                    KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
                })?;
                s.upsert_chunks(&chunks_clone, &vectors_clone)
            })
            .await
            .map_err(|e| CommandError::Internal(e.into()))?
            .map_err(|e| CommandError::Knowledge(e))?;
        }

        // 写入 BM25 索引
        {
            let mut bm25_map = self.bm25_indexes.lock().await;
            let bm25 = bm25_map
                .entry(kb_id)
                .or_insert_with(InMemoryBm25Index::new);
            bm25.add_chunks(&chunks)
                .map_err(|e| CommandError::Knowledge(e))?;
        }

        // 写入 SQLite
        let doc_record = DocRecord {
            doc_id: doc_id.clone(),
            kb_id,
            title,
            chunk_count,
            char_count: total_chars,
            created_at: chrono::Utc::now(),
        };
        self.db.add_document(&doc_record)?;

        // 更新统计
        let kb = self.db.get_knowledge_base(kb_id)?;
        if let Some(kb_record) = kb {
            self.db.update_kb_stats(
                kb_id,
                kb_record.document_count + 1,
                kb_record.total_chunks + chunk_count,
            )?;
        }

        debug!(%kb_id, %doc_id, chunk_count, "文档已添加");
        Ok(doc_record)
    }

    /// 列出知识库中的所有文档。
    pub async fn list_documents(&self, kb_id: KnowledgeBaseId) -> Result<Vec<DocRecord>, CommandError> {
        Ok(self.db.list_documents(kb_id)?)
    }

    /// 删除文档。
    pub async fn delete_document(
        &self,
        kb_id: KnowledgeBaseId,
        doc_id: &str,
    ) -> Result<(), CommandError> {
        // 从向量存储中删除
        {
            let stores = self.open_stores.lock().await;
            if let Some(store) = stores.get(&kb_id) {
                let store = store.clone();
                let doc_id_owned = doc_id.to_string();
                tokio::task::spawn_blocking(move || {
                    let s = store.lock().map_err(|e| {
                        KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
                    })?;
                    s.delete_document(&doc_id_owned)
                })
                .await
                .map_err(|e| CommandError::Internal(e.into()))?
                .map_err(|e| CommandError::Knowledge(e))?;
            }
        }

        // 从 BM25 索引中删除
        {
            let mut bm25_map = self.bm25_indexes.lock().await;
            if let Some(bm25) = bm25_map.get_mut(&kb_id) {
                bm25.remove_document(doc_id)
                    .map_err(|e| CommandError::Knowledge(e))?;
            }
        }

        // 从 SQLite 中删除
        self.db.delete_document(kb_id, doc_id)?;

        info!(%kb_id, %doc_id, "文档已删除");
        Ok(())
    }

    // ── Search ──────────────────────────────────────────────────

    /// 混合搜索：BM25 + 向量搜索 + RRF 融合。
    pub async fn hybrid_search(
        &self,
        kb_id: KnowledgeBaseId,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, CommandError> {
        let stores = self.open_stores.lock().await;
        let bm25_map = self.bm25_indexes.lock().await;

        let store = self.get_or_open_store_inner(kb_id, &stores)?;
        let bm25 = bm25_map.get(&kb_id).ok_or_else(|| {
            CommandError::Knowledge(KnowledgeError::KnowledgeBaseNotFound(kb_id.to_string()))
        })?;

        // 嵌入查询向量
        let query_vector = {
            let embedder = Arc::clone(&self.embedder);
            let query_owned = query.to_string();
            tokio::task::spawn_blocking(move || embedder.embed(&query_owned))
                .await
                .map_err(|e| CommandError::Internal(e.into()))?
                .map_err(|e| CommandError::Knowledge(e))?
        };

        // 向量搜索（同步，使用 spawn_blocking）
        let vector_results = {
            let store = store.clone();
            let query_vec = query_vector.clone();
            let candidate_limit = top_k * 2;
            tokio::task::spawn_blocking(move || {
                let s = store.lock().map_err(|e| {
                    KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
                })?;
                s.search(&query_vec, candidate_limit, None)
            })
            .await
            .map_err(|e| CommandError::Internal(e.into()))?
            .map_err(|e| CommandError::Knowledge(e))?
        };

        // BM25 搜索
        let bm25_results = bm25.search(query, top_k * 2)
            .map_err(|e| CommandError::Knowledge(e))?;

        // RRF 融合
        let fused = chat_pm_knowledge::rrf_fuse(
            &vector_results,
            &bm25_results,
            self.searcher.rrf_k,
        );

        Ok(fused.into_iter().take(top_k).collect())
    }

    /// 检索会话引用的所有知识库的上下文。
    pub async fn retrieve_context(
        &self,
        session_id: SessionId,
        query: &str,
        top_k: usize,
    ) -> Result<Vec<SearchResult>, CommandError> {
        let kb_ids = self.db.get_session_kb_refs(session_id)?;

        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }

        let mut all_results = Vec::new();
        for kb_id in &kb_ids {
            match self.hybrid_search(*kb_id, query, top_k).await {
                Ok(results) => all_results.extend(results),
                Err(e) => {
                    warn!(%kb_id, error = %e, "知识库检索失败，跳过");
                }
            }
        }

        // 按分数重新排序
        all_results.sort_by(|a, b| {
            b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal)
        });

        // 去重
        let mut seen = std::collections::HashSet::new();
        all_results.retain(|r| seen.insert(r.chunk_id.clone()));

        Ok(all_results.into_iter().take(top_k).collect())
    }

    // ── Internal Helpers ────────────────────────────────────────

    /// 获取知识库的存储目录。
    fn kb_dir(&self, kb_id: KnowledgeBaseId) -> PathBuf {
        self.stores_dir.join(kb_id.to_string())
    }

    /// 获取或打开向量存储。
    fn get_or_open_store_inner(
        &self,
        kb_id: KnowledgeBaseId,
        stores: &std::collections::HashMap<KnowledgeBaseId, Arc<StdMutex<EdgeVectorStore>>>,
    ) -> Result<Arc<StdMutex<EdgeVectorStore>>, CommandError> {
        if let Some(store) = stores.get(&kb_id) {
            return Ok(Arc::clone(store));
        }

        let shard_dir = self.kb_dir(kb_id);
        let store = if shard_dir.exists() && shard_dir.join("segments").exists() {
            EdgeVectorStore::load(&shard_dir)
        } else {
            EdgeVectorStore::new(&shard_dir, self.embedder.dimension())
        }
        .map_err(|e| CommandError::Knowledge(e))?;

        Ok(Arc::new(StdMutex::new(store)))
    }
}
