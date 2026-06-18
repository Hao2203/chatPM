mod mock;
#[cfg(feature = "onnx")]
mod onnx;

use crate::error::KnowledgeError;

pub use mock::MockEmbedder;
#[cfg(feature = "onnx")]
pub use onnx::OnnxEmbedder;

/// 文本嵌入 trait。
///
/// 抽象嵌入模型的接口，允许替换不同的实现（ONNX 本地模型、外部 API、Mock 等）。
pub trait Embed: Send + Sync {
    /// 对单个文本生成嵌入向量。
    fn embed(&self, text: &str) -> Result<Vec<f32>, KnowledgeError>;

    /// 对批量文本生成嵌入向量（默认逐个调用，可重写以提升效率）。
    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, KnowledgeError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    /// 返回嵌入向量的维度。
    fn dimension(&self) -> usize;
}
