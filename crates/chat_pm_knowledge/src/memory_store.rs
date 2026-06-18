use std::collections::HashMap;
use std::sync::Mutex;

use crate::chunk::DocumentChunk;
use crate::error::KnowledgeError;
use crate::vector_store::{ScoredPoint, SearchResult, VectorStore};

/// 基于内存 HashMap + 暴力余弦相似度的向量存储实现。
///
/// 用于测试，不依赖 qdrant-edge。
pub struct InMemoryVectorStore {
    /// chunk_id -> (vector, chunk_info)
    entries: Mutex<HashMap<String, (Vec<f32>, ScoredPoint)>>,
    /// 向量维度
    dimension: usize,
}

impl InMemoryVectorStore {
    /// 创建新的内存向量存储。
    pub fn new(dimension: usize) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            dimension,
        }
    }
}

impl VectorStore for InMemoryVectorStore {
    fn upsert_chunks(
        &self,
        chunks: &[DocumentChunk],
        vectors: &[Vec<f32>],
    ) -> Result<(), KnowledgeError> {
        if chunks.len() != vectors.len() {
            return Err(KnowledgeError::VectorStoreError(
                "chunks and vectors length mismatch".to_string(),
            ));
        }

        let mut entries = self.entries.lock().map_err(|e| {
            KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
        })?;

        for (chunk, vector) in chunks.iter().zip(vectors.iter()) {
            let point = ScoredPoint {
                chunk_id: chunk.chunk_id.to_string(),
                document_id: chunk.document_id.clone(),
                chunk_index: chunk.chunk_index,
                content: chunk.content.clone(),
                score: 0.0,
            };
            entries.insert(chunk.chunk_id.to_string(), (vector.clone(), point));
        }

        Ok(())
    }

    fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter_doc_id: Option<&str>,
    ) -> Result<Vec<SearchResult>, KnowledgeError> {
        let entries = self.entries.lock().map_err(|e| {
            KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
        })?;

        let mut scored: Vec<SearchResult> = entries
            .iter()
            .filter(|(_, (_, point))| {
                if let Some(doc_id) = filter_doc_id {
                    point.document_id == doc_id
                } else {
                    true
                }
            })
            .map(|(_, (vec, point))| {
                let similarity = cosine_similarity(query_vector, vec);
                SearchResult {
                    chunk_id: point.chunk_id.clone(),
                    document_id: point.document_id.clone(),
                    chunk_index: point.chunk_index,
                    content: point.content.clone(),
                    score: similarity,
                }
            })
            .collect();

        // 按分数降序排序
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // 截取 top-k
        scored.truncate(limit);

        Ok(scored)
    }

    fn delete_document(&self, document_id: &str) -> Result<usize, KnowledgeError> {
        let mut entries = self.entries.lock().map_err(|e| {
            KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
        })?;

        let before = entries.len();
        entries.retain(|_, (_, point)| point.document_id != document_id);
        Ok(before - entries.len())
    }

    fn clear(&self) -> Result<(), KnowledgeError> {
        let mut entries = self.entries.lock().map_err(|e| {
            KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e))
        })?;
        entries.clear();
        Ok(())
    }
}

/// 计算两个向量的余弦相似度。
pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    let dot_product: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        dot_product / (norm_a * norm_b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkId;
    use crate::embed::Embed;

    fn make_chunk(content: &str, doc_id: &str, index: usize) -> DocumentChunk {
        DocumentChunk {
            chunk_id: ChunkId::new(),
            knowledge_base_id: "test_kb".to_string(),
            document_id: doc_id.to_string(),
            chunk_index: index,
            content: content.to_string(),
            char_count: content.len(),
        }
    }

    #[test]
    fn upsert_and_search() {
        let store = InMemoryVectorStore::new(8);
        let embedder = crate::mock_embed::MockEmbedder::new(8);

        let chunks = vec![
            make_chunk("Rust is a systems programming language", "doc1", 0),
            make_chunk("Python is used for data science", "doc1", 1),
            make_chunk("JavaScript is for frontend development", "doc2", 0),
        ];

        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();

        store.upsert_chunks(&chunks, &vectors).unwrap();

        // 搜索与 doc1/chunk0 完全相同文本 → 应该得到最高分
        let query_vec = embedder.embed("Rust is a systems programming language").unwrap();
        let results = store.search(&query_vec, 2, None).unwrap();

        assert_eq!(results.len(), 2);
        // 完全匹配的文本余弦相似度应为 1.0
        assert!((results[0].score - 1.0).abs() < 0.001,
            "expected score ~1.0, got {}", results[0].score);
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn search_with_document_filter() {
        let store = InMemoryVectorStore::new(16);
        let embedder = crate::mock_embed::MockEmbedder::new(16);

        let chunks = vec![
            make_chunk("Rust content", "doc1", 0),
            make_chunk("Python content", "doc2", 0),
        ];

        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();

        store.upsert_chunks(&chunks, &vectors).unwrap();

        let query_vec = embedder.embed("content").unwrap();
        let results = store.search(&query_vec, 10, Some("doc1")).unwrap();

        // 只返回 doc1 的结果
        for r in &results {
            assert_eq!(r.document_id, "doc1");
        }
    }

    #[test]
    fn delete_document() {
        let store = InMemoryVectorStore::new(16);
        let embedder = crate::mock_embed::MockEmbedder::new(16);

        let chunks = vec![
            make_chunk("Rust content", "doc1", 0),
            make_chunk("Python content", "doc2", 0),
        ];

        let vectors: Vec<Vec<f32>> = chunks
            .iter()
            .map(|c| embedder.embed(&c.content).unwrap())
            .collect();

        store.upsert_chunks(&chunks, &vectors).unwrap();

        let deleted = store.delete_document("doc1").unwrap();
        assert_eq!(deleted, 1);

        let query_vec = embedder.embed("content").unwrap();
        let results = store.search(&query_vec, 10, None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].document_id, "doc2");
    }

    #[test]
    fn clear_removes_all() {
        let store = InMemoryVectorStore::new(16);
        let embedder = crate::mock_embed::MockEmbedder::new(16);

        let chunks = vec![make_chunk("test", "doc1", 0)];
        let vectors = vec![embedder.embed("test").unwrap()];

        store.upsert_chunks(&chunks, &vectors).unwrap();
        store.clear().unwrap();

        let query_vec = embedder.embed("test").unwrap();
        let results = store.search(&query_vec, 10, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 0.001);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 0.001);
    }
}
