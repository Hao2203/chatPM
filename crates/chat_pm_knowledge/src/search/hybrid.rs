use crate::embed::Embed;
use crate::error::KnowledgeError;
use crate::store::{SearchResult, VectorStore};

use super::bm25::Bm25Index;

use super::rrf::rrf_fuse;

/// 混合检索器：同时执行 BM25 关键词搜索和向量语义搜索，通过 RRF 融合结果。
pub struct HybridSearcher {
    /// RRF 平滑常数 k，默认 60。
    pub rrf_k: f64,
    /// 每种搜索的候选数倍数（limit * search_multiplier 候选，然后 RRF 融合取 top-k）。
    pub search_multiplier: usize,
}

impl Default for HybridSearcher {
    fn default() -> Self {
        Self {
            rrf_k: 60.0,
            search_multiplier: 2,
        }
    }
}

impl HybridSearcher {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_rrf_k(mut self, k: f64) -> Self {
        self.rrf_k = k;
        self
    }

    /// 执行混合检索。
    ///
    /// # 参数
    /// - `query`: 用户查询文本
    /// - `embedder`: 嵌入模型
    /// - `vector_store`: 向量存储
    /// - `bm25_index`: BM25 索引
    /// - `limit`: 最终返回的结果数
    ///
    /// # 返回
    /// RRF 融合排序后的 Top-K 结果
    pub fn search(
        &self,
        query: &str,
        embedder: &dyn Embed,
        vector_store: &dyn VectorStore,
        bm25_index: &dyn Bm25Index,
        limit: usize,
    ) -> Result<Vec<SearchResult>, KnowledgeError> {
        let candidate_limit = limit * self.search_multiplier;

        // 并行执行两种搜索以提升效率 — TODO: 使用 tokio::join! 在 async 上下文中
        let vector_results: Vec<SearchResult> = {
            let query_vector = embedder.embed(query)?;
            vector_store.search(&query_vector, candidate_limit, None)?
        };

        let bm25_results: Vec<SearchResult> = bm25_index.search(query, candidate_limit)?;

        // RRF 融合
        let mut fused = rrf_fuse(&vector_results, &bm25_results, self.rrf_k);

        // 截取 top-k
        fused.truncate(limit);

        Ok(fused)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::{ChunkId, DocumentChunk, DocumentId};
    use crate::embed::MockEmbedder;
    use crate::knowledge_base::KnowledgeBaseId;
    use crate::search::{Bm25Index, InMemoryBm25Index};
    use crate::store::InMemoryVectorStore;

    fn make_chunk(content: &str, doc_id: &str, index: usize) -> DocumentChunk {
        DocumentChunk {
            chunk_id: ChunkId::new(),
            knowledge_base_id: KnowledgeBaseId::new(),
            document_id: DocumentId::new(doc_id),
            chunk_index: index,
            content: content.to_string(),
            char_count: content.len(),
        }
    }

    #[test]
    fn hybrid_search_combines_both_sources() {
        let dim = 64;
        let embedder = MockEmbedder::new(dim);
        let vector_store = InMemoryVectorStore::new(dim);
        let mut bm25_index = InMemoryBm25Index::new();

        // 添加测试数据
        let chunks = vec![
            make_chunk("Rust 是一门现代系统编程语言，具有内存安全特性", "doc1", 0),
            make_chunk("Python 是数据科学和机器学习领域的主流语言", "doc2", 0),
            make_chunk("Go 语言由 Google 开发，专注于并发编程", "doc3", 0),
        ];

        // 生成向量并写入向量存储
        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();
        vector_store.upsert_chunks(&chunks, &vectors).unwrap();

        // 写入 BM25 索引
        bm25_index.add_chunks(&chunks).unwrap();

        let searcher = HybridSearcher::default();

        // 搜索系统编程相关内容
        let results = searcher
            .search("系统编程", &embedder, &vector_store, &bm25_index, 3)
            .unwrap();

        assert!(!results.is_empty());
        // "Rust" 相关内容应该在第一位（同时匹配关键词"系统编程"和语义相似）
    }

    #[test]
    fn hybrid_search_respects_limit() {
        let dim = 16;
        let embedder = MockEmbedder::new(dim);
        let vector_store = InMemoryVectorStore::new(dim);
        let mut bm25_index = InMemoryBm25Index::new();

        let mut chunks = Vec::new();
        for i in 0..10 {
            chunks.push(make_chunk(
                &format!("文档 {} 的内容是关于 Rust 编程的详细介绍", i),
                &format!("doc{}", i),
                0,
            ));
        }

        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();
        vector_store.upsert_chunks(&chunks, &vectors).unwrap();
        bm25_index.add_chunks(&chunks).unwrap();

        let searcher = HybridSearcher::default();

        let results = searcher
            .search("Rust 编程", &embedder, &vector_store, &bm25_index, 3)
            .unwrap();

        assert_eq!(results.len(), 3);
    }

    #[test]
    fn hybrid_search_deduplicates() {
        let dim = 16;
        let embedder = MockEmbedder::new(dim);
        let vector_store = InMemoryVectorStore::new(dim);
        let mut bm25_index = InMemoryBm25Index::new();

        // 只有一个文档
        let chunks = vec![make_chunk("Rust 编程语言", "doc1", 0)];

        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();
        vector_store.upsert_chunks(&chunks, &vectors).unwrap();
        bm25_index.add_chunks(&chunks).unwrap();

        let searcher = HybridSearcher::default();

        // 搜索应该产生去重后的结果（同一块同时出现在两边，RRF 融合后只保留一个）
        let results = searcher
            .search("Rust", &embedder, &vector_store, &bm25_index, 10)
            .unwrap();

        // 所有出现的 chunk_id 应该唯一
        let mut ids: Vec<ChunkId> = results.iter().map(|r| r.chunk_id).collect();
        ids.sort();
        let mut deduped = ids.clone();
        deduped.dedup();
        assert_eq!(ids.len(), deduped.len(), "结果集中不应有重复的 chunk_id");
    }
}
