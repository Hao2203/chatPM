use std::path::Path;

use super::Embed;
use crate::error::KnowledgeError;

/// 基于 ONNX Runtime 的嵌入器（占位实现）。
///
/// 使用 all-MiniLM-L6-v2 模型（384 维）。
/// 仅在启用 `onnx` feature 时编译。
#[cfg(not(feature = "onnx"))]
pub struct OnnxEmbedder {
    _placeholder: (),
}

#[cfg(not(feature = "onnx"))]
impl OnnxEmbedder {
    pub fn new(_model_dir: &Path) -> Result<Self, KnowledgeError> {
        Err(KnowledgeError::EmbeddingError(
            "ONNX 嵌入器未启用，请使用 'onnx' feature 编译".to_string(),
        ))
    }
}

#[cfg(not(feature = "onnx"))]
impl Embed for OnnxEmbedder {
    fn embed(&self, _text: &str) -> Result<Vec<f32>, KnowledgeError> {
        Err(KnowledgeError::EmbeddingError("ONNX 未启用".to_string()))
    }

    fn dimension(&self) -> usize {
        0
    }
}

// ===== feature = "onnx" 的实现 =====

#[cfg(feature = "onnx")]
use std::sync::Mutex;

#[cfg(feature = "onnx")]
use ort::{inputs, session::Session, value::Tensor};

#[cfg(feature = "onnx")]
pub struct OnnxEmbedder {
    session: Mutex<Session>,
    tokenizer: tokenizers::Tokenizer,
    dimension: usize,
}

#[cfg(feature = "onnx")]
impl OnnxEmbedder {
    pub fn new(model_dir: &Path) -> Result<Self, KnowledgeError> {
        let model_path = model_dir.join("model.onnx");
        let tokenizer_path = model_dir.join("tokenizer.json");

        if !model_path.exists() {
            return Err(KnowledgeError::EmbeddingError(format!(
                "ONNX 模型文件不存在: {}",
                model_path.display()
            )));
        }
        if !tokenizer_path.exists() {
            return Err(KnowledgeError::EmbeddingError(format!(
                "Tokenizer 配置文件不存在: {}",
                tokenizer_path.display()
            )));
        }

        let session = Session::builder()
            .map_err(|e| KnowledgeError::EmbeddingError(format!("创建 ONNX session 失败: {}", e)))?
            .commit_from_file(&model_path)
            .map_err(|e| KnowledgeError::EmbeddingError(format!("加载 ONNX 模型失败: {}", e)))?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path)
            .map_err(|e| KnowledgeError::EmbeddingError(format!("加载 tokenizer 失败: {}", e)))?;

        let dimension = 384;

        let embedder = Self {
            session: Mutex::new(session),
            tokenizer,
            dimension,
        };

        // 预热
        let _ = embedder.embed("warmup")?;

        Ok(embedder)
    }

    fn run_inference(
        session: &Mutex<Session>,
        tokenizer: &tokenizers::Tokenizer,
        text: &str,
    ) -> Result<Vec<f32>, KnowledgeError> {
        let encoding = tokenizer
            .encode(text, true)
            .map_err(|e| KnowledgeError::EmbeddingError(format!("tokenize 失败: {}", e)))?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        let token_type_ids: Vec<i64> = encoding.get_type_ids().iter().map(|&t| t as i64).collect();

        let seq_len = input_ids.len();

        // 使用 tuple (shape, data) 创建张量，无需 ndarray 依赖
        let input_ids_tensor = Tensor::from_array(([1usize, seq_len], input_ids)).map_err(|e| {
            KnowledgeError::EmbeddingError(format!("创建 input_ids tensor 失败: {}", e))
        })?;

        let attention_mask_tensor = Tensor::from_array(([1usize, seq_len], attention_mask))
            .map_err(|e| {
                KnowledgeError::EmbeddingError(format!("创建 attention_mask tensor 失败: {}", e))
            })?;

        let token_type_ids_tensor = Tensor::from_array(([1usize, seq_len], token_type_ids))
            .map_err(|e| {
                KnowledgeError::EmbeddingError(format!("创建 token_type_ids tensor 失败: {}", e))
            })?;

        let mut session = session
            .lock()
            .map_err(|e| KnowledgeError::EmbeddingError(format!("Session lock poisoned: {}", e)))?;

        let outputs = session
            .run(inputs![
                input_ids_tensor,
                attention_mask_tensor,
                token_type_ids_tensor
            ])
            .map_err(|e| KnowledgeError::EmbeddingError(format!("推理失败: {}", e)))?;

        let (_shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| KnowledgeError::EmbeddingError(format!("提取 tensor 失败: {}", e)))?;

        // Mean pooling
        let embedding_dim = data.len() / seq_len;
        let mut embedding = vec![0.0f32; embedding_dim];
        for (i, val) in data.iter().enumerate() {
            embedding[i % embedding_dim] += val;
        }
        for val in &mut embedding {
            *val /= seq_len as f32;
        }

        // L2 归一化
        let norm: f32 = embedding.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for val in &mut embedding {
                *val /= norm;
            }
        }

        Ok(embedding)
    }
}

#[cfg(feature = "onnx")]
impl Embed for OnnxEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>, KnowledgeError> {
        Self::run_inference(&self.session, &self.tokenizer, text)
    }

    fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, KnowledgeError> {
        texts.iter().map(|t| self.embed(t)).collect()
    }

    fn dimension(&self) -> usize {
        self.dimension
    }
}
