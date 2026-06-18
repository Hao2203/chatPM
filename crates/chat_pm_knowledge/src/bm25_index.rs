use crate::chunk::DocumentChunk;
use crate::error::KnowledgeError;
use crate::vector_store::SearchResult;

/// BM25 关键词搜索索引 trait。
///
/// 提供精确的关键词匹配能力，与向量语义搜索互补。
pub trait Bm25Index: Send + Sync {
    /// 向索引中添加文本块。
    fn add_chunks(&mut self, chunks: &[DocumentChunk]) -> Result<(), KnowledgeError>;

    /// 从索引中移除指定文档的所有块。
    fn remove_document(&mut self, document_id: &str) -> Result<usize, KnowledgeError>;

    /// 关键词搜索，返回 Top-K 结果。
    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KnowledgeError>;

    /// 清空索引。
    fn clear(&mut self) -> Result<(), KnowledgeError>;
}
