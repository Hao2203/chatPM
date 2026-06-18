use super::Embed;
use crate::error::KnowledgeError;

/// Mock 嵌入器，返回基于文本长度的确定性向量。
///
/// 用于测试，不依赖 ONNX Runtime 或外部 API。
pub struct MockEmbedder {
    dimension: usize,
}

impl MockEmbedder {
    /// 创建指定维度的 Mock 嵌入器。
    pub fn new(dimension: usize) -> Self {
        Self { dimension }
    }
}

impl Embed for MockEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, KnowledgeError> {
        // 基于文本内容的简单确定性哈希向量
        let mut vec = vec![0.0f32; self.dimension];
        for (i, byte) in text.bytes().enumerate() {
            let idx = i % self.dimension;
            vec[idx] += byte as f32 / 255.0;
        }
        // 归一化
        let norm: f32 = vec.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vec {
                *v /= norm;
            }
        }
        Ok(vec)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, KnowledgeError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedder_returns_correct_dimension() {
        let embedder = MockEmbedder::new(128);
        assert_eq!(embedder.dimension(), 128);
    }

    #[test]
    fn mock_embedder_returns_vector() {
        let embedder = MockEmbedder::new(16);
        let vec = embedder.embed("test text").unwrap();
        assert_eq!(vec.len(), 16);
    }

    #[test]
    fn mock_embedder_same_text_same_vector() {
        let embedder = MockEmbedder::new(16);
        let v1 = embedder.embed("hello").unwrap();
        let v2 = embedder.embed("hello").unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn mock_embedder_different_text_different_vector() {
        let embedder = MockEmbedder::new(64);
        let v1 = embedder.embed("hello").unwrap();
        let v2 = embedder.embed("world").unwrap();
        assert_ne!(v1, v2);
    }
}
