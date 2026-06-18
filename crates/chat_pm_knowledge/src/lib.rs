//! # chat_pm_knowledge — 知识库引擎
//!
//! 为 chatPM 提供本地优先的资料库能力，包括：
//!
//! * **文档分块** — 递归文本分割，支持中英文混合
//! * **语义向量** — 可插拔嵌入模型（ONNX 本地 / Mock 测试）
//! * **关键词检索** — 自实现 BM25，中英文分词
//! * **混合检索** — RRF 融合向量 + 关键词结果
//!
//! ## 模块结构
//!
//! ```text
//! embed/    → 嵌入模型层（Embed trait + 实现）
//! store/    → 存储后端层（向量 + BM25 索引）
//! search/   → 搜索组合层（RRF 融合 + 混合检索）
//! ```
//!
//! 基础类型（`error`、`chunk`、`knowledge_base`）平铺在 crate 根层。

// ── 基础类型 ──────────────────────────────────────────────────

pub mod chunk;
pub mod error;
pub mod knowledge_base;

// ── 嵌入模型层 ────────────────────────────────────────────────

pub mod embed;

// ── 存储后端层 ────────────────────────────────────────────────

pub mod store;

// ── 搜索组合层 ────────────────────────────────────────────────

pub mod search;

// ── Re-exports ────────────────────────────────────────────────

pub use chunk::{ChunkConfig, ChunkId, DocumentChunk, DocumentId, chunk_text};
#[cfg(feature = "onnx")]
pub use embed::OnnxEmbedder;
pub use embed::{Embed, MockEmbedder};
pub use error::KnowledgeError;
pub use knowledge_base::{KnowledgeBase, KnowledgeBaseId, KnowledgeBaseName};
pub use search::{Bm25Index, Bm25Params, HybridSearcher, InMemoryBm25Index, rrf_fuse};
pub use store::{EdgeVectorStore, InMemoryVectorStore, ScoredPoint, SearchResult, VectorStore};
