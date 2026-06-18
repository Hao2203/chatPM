mod edge;
mod memory;

use crate::chunk::{ChunkId, DocumentChunk, DocumentId};
use crate::error::KnowledgeError;

pub use edge::EdgeVectorStore;
pub use memory::InMemoryVectorStore;

/// 搜索结果，包含文本块内容和相关性分数。
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// 文本块的唯一标识。
    pub chunk_id: ChunkId,
    /// 所属文档标识。
    pub document_id: DocumentId,
    /// 在文档中的位置索引。
    pub chunk_index: usize,
    /// 文本内容。
    pub content: String,
    /// 相关性分数（余弦相似度、BM25 分数或 RRF 融合分数）。
    pub score: f32,
}

/// 内部使用的带分数点。
#[derive(Debug, Clone)]
pub struct ScoredPoint {
    pub chunk_id: ChunkId,
    pub document_id: DocumentId,
    pub chunk_index: usize,
    pub content: String,
    pub score: f32,
}

/// 向量存储 trait。
///
/// 抽象向量数据库的操作接口，允许替换底层实现（qdrant-edge、内存 HashMap 等）。
pub trait VectorStore: Send + Sync {
    /// 批量插入或更新文本块及其向量。
    fn upsert_chunks(
        &self,
        chunks: &[DocumentChunk],
        vectors: &[Vec<f32>],
    ) -> Result<(), KnowledgeError>;

    /// 向量相似度搜索，返回 Top-K 结果。
    fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter_doc_id: Option<&DocumentId>,
    ) -> Result<Vec<SearchResult>, KnowledgeError>;

    /// 删除指定文档的所有块。
    fn delete_document(&self, document_id: &DocumentId) -> Result<usize, KnowledgeError>;

    /// 清空整个存储。
    fn clear(&self) -> Result<(), KnowledgeError>;

    /// 强制持久化到磁盘（如底层支持）。
    fn flush(&self) -> Result<(), KnowledgeError> {
        Ok(())
    }
}
