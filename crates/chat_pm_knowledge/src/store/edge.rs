use std::path::Path;

#[cfg(not(feature = "qdrant"))]
use std::path::PathBuf;
#[cfg(not(feature = "qdrant"))]
use std::sync::Mutex;

use crate::chunk::{DocumentChunk, DocumentId};
use crate::error::KnowledgeError;

/// 基于 `qdrant-edge` 的向量存储（占位实现）。
///
/// 当前使用内存存储替代，待 qdrant-edge API 稳定后切换为真正的磁盘存储。
/// 此类型保留了磁盘路径接口，确保将来切换时 API 不变。
#[cfg(not(feature = "qdrant"))]
use super::memory::InMemoryVectorStore as InnerStore;
#[cfg(not(feature = "qdrant"))]
use super::VectorStore;

#[cfg(not(feature = "qdrant"))]
pub struct EdgeVectorStore {
    inner: Mutex<InnerStore>,
    path: PathBuf,
}

#[cfg(feature = "qdrant")]
pub struct EdgeVectorStore {
    // TODO: 使用 qdrant_edge::EdgeShard 实现
    _placeholder: (),
}

#[cfg(not(feature = "qdrant"))]
impl EdgeVectorStore {
    /// 在指定路径创建新的向量存储。
    pub fn new(path: &Path, dimension: usize) -> Result<Self, KnowledgeError> {
        // 确保目录存在
        if !path.exists() {
            std::fs::create_dir_all(path)
                .map_err(|e| KnowledgeError::VectorStoreError(format!("创建目录失败: {}", e)))?;
        }

        Ok(Self {
            inner: Mutex::new(InnerStore::new(dimension)),
            path: path.to_path_buf(),
        })
    }

    /// 打开已有的向量存储。
    pub fn load(path: &Path) -> Result<Self, KnowledgeError> {
        // 目前使用内存存储，加载时重新创建
        // 后续切换到 qdrant-edge 后实现真正的磁盘加载
        let dimension = 384; // 默认维度，后续从配置读取
        Self::new(path, dimension)
    }

    /// 返回存储路径。
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 批量插入或更新块及其向量。
    pub fn upsert_chunks(
        &self,
        chunks: &[DocumentChunk],
        vectors: &[Vec<f32>],
    ) -> Result<(), KnowledgeError> {
        let store = self
            .inner
            .lock()
            .map_err(|e| KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e)))?;
        store.upsert_chunks(chunks, vectors)
    }

    /// 向量相似度搜索。
    pub fn search(
        &self,
        query_vector: &[f32],
        limit: usize,
        filter_doc_id: Option<&DocumentId>,
    ) -> Result<Vec<super::SearchResult>, KnowledgeError> {
        let store = self
            .inner
            .lock()
            .map_err(|e| KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e)))?;
        store.search(query_vector, limit, filter_doc_id)
    }

    /// 删除指定文档的所有块。
    pub fn delete_document(&self, document_id: &DocumentId) -> Result<usize, KnowledgeError> {
        let store = self
            .inner
            .lock()
            .map_err(|e| KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e)))?;
        store.delete_document(document_id)
    }

    /// 清空所有数据。
    pub fn clear(&self) -> Result<(), KnowledgeError> {
        let store = self
            .inner
            .lock()
            .map_err(|e| KnowledgeError::VectorStoreError(format!("Lock poisoned: {}", e)))?;
        store.clear()
    }

    /// 强制持久化（当前为内存存储，无操作）。
    pub fn flush(&self) -> Result<(), KnowledgeError> {
        Ok(())
    }
}

#[cfg(feature = "qdrant")]
impl EdgeVectorStore {
    pub fn new(_path: &Path, _dimension: usize) -> Result<Self, KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn load(_path: &Path) -> Result<Self, KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn path(&self) -> &Path {
        Path::new("")
    }

    pub fn upsert_chunks(
        &self,
        _chunks: &[DocumentChunk],
        _vectors: &[Vec<f32>],
    ) -> Result<(), KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn search(
        &self,
        _query_vector: &[f32],
        _limit: usize,
        _filter_doc_id: Option<&DocumentId>,
    ) -> Result<Vec<super::SearchResult>, KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn delete_document(&self, _document_id: &DocumentId) -> Result<usize, KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn clear(&self) -> Result<(), KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }

    pub fn flush(&self) -> Result<(), KnowledgeError> {
        Err(KnowledgeError::VectorStoreError(
            "qdrant-edge 集成尚未实现".to_string(),
        ))
    }
}
