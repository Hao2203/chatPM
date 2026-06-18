/// 知识库领域的错误类型。
///
/// 仅包含业务逻辑违反，不包含 I/O 或基础设施故障。
#[derive(Debug, Clone, thiserror::Error)]
pub enum KnowledgeError {
    #[error("知识库 '{0}' 不存在")]
    KnowledgeBaseNotFound(crate::knowledge_base::KnowledgeBaseId),

    #[error("知识库名称 '{0}' 已存在")]
    KnowledgeBaseAlreadyExists(String),

    #[error("文档内容太短，至少需要 {0} 个字符")]
    DocumentTooShort(usize),

    #[error("嵌入错误: {0}")]
    EmbeddingError(String),

    #[error("向量存储错误: {0}")]
    VectorStoreError(String),

    #[error("BM25 索引错误: {0}")]
    Bm25Error(String),
}
