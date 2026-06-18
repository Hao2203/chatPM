use std::path::Path;

use crate::embed::Embed;
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
pub struct OnnxEmbedder {
    session: ort::Session,
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

        let session = ort::Session::builder()
            .map_err(|e| {
                KnowledgeError::EmbeddingError(format!("创建 ONNX session 失败: {}", e))
            })?
            .commit_from_file(&model_path)
            .map_err(|e| {
                KnowledgeError::EmbeddingError(format!("加载 ONNX 模型失败: {}", e))
            })?;

        let tokenizer = tokenizers::Tokenizer::from_file(&tokenizer_path).map_err(|e| {
            KnowledgeError::EmbeddingError(format!("加载 tokenizer 失败: {}", e))
        })?;

        let dimension = 384;

        // 预热
        let _ = Self::run_inference(&session, &tokenizer, "warmup")?;

        Ok(Self {
            session,
            tokenizer,
            dimension,
        })
    }

    fn run_inference(
        session: &ort::Session,
        tokenizer: &tokenizers::Tokenizer,
        text: &str,
    ) -> Result<Vec<f32>, KnowledgeError> {
        let encoding = tokenizer.encode(text, true).map_err(|e| {
            KnowledgeError::EmbeddingError(format!("tokenize 失败: {}", e))
        })?;

        let input_ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        let token_type_ids: Vec<i64> = encoding
            .get_type_ids()
            .iter()
            .map(|&t| t as i64)
            .collect();

        let seq_len = input_ids.len();

        let input_ids_array = ndarray::Array2::from_shape_vec((1, seq_len), input_ids)
            .map_err(|e| KnowledgeError::EmbeddingError(format!("reshape 失败: {}", e)))?;

        let attention_mask_array =
            ndarray::Array2::from_shape_vec((1, seq_len), attention_mask)
                .map_err(|e| KnowledgeError::EmbeddingError(format!("reshape 失败: {}", e)))?;

        let token_type_ids_array =
            ndarray::Array2::from_shape_vec((1, seq_len), token_type_ids)
                .map_err(|e| KnowledgeError::EmbeddingError(format!("reshape 失败: {}", e)))?;

        let input_ids_tensor = ort::Value::from_array(
            session
                .inputs
                .first()
                .ok_or_else(|| KnowledgeError::EmbeddingError("模型没有输入".to_string()))?,
            input_ids_array,
        )
        .map_err(|e| {
            KnowledgeError::EmbeddingError(format!("创建 input_ids tensor 失败: {}", e))
        })?;

        let attention_mask_tensor = ort::Value::from_array(
            &session.inputs[1],
            attention_mask_array,
        )
        .map_err(|e| {
            KnowledgeError::EmbeddingError(format!("创建 attention_mask tensor 失败: {}", e))
        })?;

        let token_type_ids_tensor = ort::Value::from_array(
            &session.inputs[2],
            token_type_ids_array,
        )
        .map_err(|e| {
            KnowledgeError::EmbeddingError(format!("创建 token_type_ids tensor 失败: {}", e))
        })?;

        let outputs = session
            .run(vec![
                input_ids_tensor,
                attention_mask_tensor,
                token_type_ids_tensor,
            ])
            .map_err(|e| KnowledgeError::EmbeddingError(format!("推理失败: {}", e)))?;

        let output = outputs
            .first()
            .ok_or_else(|| KnowledgeError::EmbeddingError("模型没有输出".to_string()))?;

        let data: Vec<f32> = output
            .try_extract_tensor::<f32>()
            .map_err(|e| KnowledgeError::EmbeddingError(format!("提取 tensor 失败: {}", e)))?
            .iter()
            .copied()
            .collect();

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
