use std::collections::HashMap;

use crate::bm25_index::Bm25Index;
use crate::chunk::DocumentChunk;
use crate::error::KnowledgeError;
use crate::vector_store::SearchResult;

/// BM25 参数。
#[derive(Debug, Clone)]
pub struct Bm25Params {
    /// 词频饱和度参数 k1，默认 1.5。
    pub k1: f32,
    /// 文档长度归一化参数 b，默认 0.75。
    pub b: f32,
}

impl Default for Bm25Params {
    fn default() -> Self {
        Self { k1: 1.5, b: 0.75 }
    }
}

/// 基于自实现 BM25 的内存关键词搜索索引。
///
/// 不依赖外部 BM25 crate，算法直接实现，稳定可控。
pub struct InMemoryBm25Index {
    /// 所有已索引的文本块。
    chunks: Vec<DocumentChunk>,
    /// chunk_id → 分词结果（词 → 词频）
    term_freqs: HashMap<String, HashMap<String, u32>>,
    /// chunk_id → 文档长度（词数）
    doc_lengths: HashMap<String, f32>,
    /// 语料库中每个词出现的文档数
    doc_freqs: HashMap<String, u32>,
    /// 平均文档长度
    avgdl: f32,
    /// BM25 参数
    params: Bm25Params,
}

impl InMemoryBm25Index {
    /// 创建新的空 BM25 索引。
    pub fn new() -> Self {
        Self {
            chunks: Vec::new(),
            term_freqs: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_freqs: HashMap::new(),
            avgdl: 0.0,
            params: Bm25Params::default(),
        }
    }

    pub fn with_params(params: Bm25Params) -> Self {
        Self {
            chunks: Vec::new(),
            term_freqs: HashMap::new(),
            doc_lengths: HashMap::new(),
            doc_freqs: HashMap::new(),
            avgdl: 0.0,
            params,
        }
    }

    /// 重新计算文档频率和平均长度。
    fn recompute_stats(&mut self) {
        self.doc_freqs.clear();

        // 计算每个词的文档频率
        for (_, tf) in &self.term_freqs {
            for term in tf.keys() {
                *self.doc_freqs.entry(term.clone()).or_insert(0) += 1;
            }
        }

        // 计算平均文档长度
        let total_len: f32 = self.doc_lengths.values().sum();
        let doc_count = self.doc_lengths.len();
        self.avgdl = if doc_count > 0 {
            total_len / doc_count as f32
        } else {
            0.0
        };
    }

    /// 分词：将文本分割为词条。
    ///
    /// 对于英文等空格分隔的语言：按空白和标点分割为小写词。
    /// 对于中文等无空格语言：使用字符级 2-gram。
    fn tokenize(text: &str) -> Vec<String> {
        let mut tokens = Vec::new();

        // 先按空白和 ASCII 标点分割
        let segments: Vec<&str> = text
            .split(|c: char| c.is_whitespace() || c.is_ascii_punctuation())
            .filter(|s| !s.is_empty())
            .collect();

        for segment in segments {
            if segment.chars().any(|c| c.is_ascii_alphabetic()) {
                // 英文段：保留小写形式
                tokens.push(segment.to_lowercase());
            } else {
                // 中文等无空格段：使用字符 2-gram
                let chars: Vec<char> = segment.chars().collect();
                if chars.len() == 1 {
                    tokens.push(chars[0].to_string());
                } else {
                    for window in chars.windows(2) {
                        tokens.push(window.iter().collect::<String>());
                    }
                }
            }
        }

        tokens
    }

    /// 计算 BM25 分数。
    ///
    /// BM25(D, Q) = Σ IDF(qi) * (f(qi,D) * (k1+1)) / (f(qi,D) + k1 * (1-b + b*|D|/avgdl))
    fn bm25_score(&self, query_terms: &[String], chunk_id: &str) -> f32 {
        let tf = match self.term_freqs.get(chunk_id) {
            Some(tf) => tf,
            None => return 0.0,
        };
        let doc_len = self.doc_lengths.get(chunk_id).copied().unwrap_or(0.0);

        let doc_count = self.doc_lengths.len() as f32;
        let k1 = self.params.k1;
        let b = self.params.b;

        query_terms
            .iter()
            .map(|term| {
                let f = *tf.get(term).unwrap_or(&0) as f32;
                if f == 0.0 {
                    return 0.0;
                }
                let n = *self.doc_freqs.get(term).unwrap_or(&0) as f32;

                // IDF
                let idf = ((doc_count - n + 0.5) / (n + 0.5) + 1.0).ln().max(0.0);

                // TF component
                let tf_component =
                    (f * (k1 + 1.0)) / (f + k1 * (1.0 - b + b * doc_len / self.avgdl.max(1.0)));

                idf * tf_component
            })
            .sum()
    }
}

impl Default for InMemoryBm25Index {
    fn default() -> Self {
        Self::new()
    }
}

impl Bm25Index for InMemoryBm25Index {
    fn add_chunks(&mut self, chunks: &[DocumentChunk]) -> Result<(), KnowledgeError> {
        for chunk in chunks {
            let terms = Self::tokenize(&chunk.content);
            let doc_len = terms.len() as f32;

            // 计算词频
            let mut tf: HashMap<String, u32> = HashMap::new();
            for term in &terms {
                *tf.entry(term.clone()).or_insert(0) += 1;
            }

            let chunk_id = chunk.chunk_id.to_string();
            self.term_freqs.insert(chunk_id.clone(), tf);
            self.doc_lengths.insert(chunk_id, doc_len);
            self.chunks.push(chunk.clone());
        }

        self.recompute_stats();
        Ok(())
    }

    fn remove_document(&mut self, document_id: &str) -> Result<usize, KnowledgeError> {
        let before = self.chunks.len();

        // 找到要移除的 chunk_id 列表
        let ids_to_remove: Vec<String> = self
            .chunks
            .iter()
            .filter(|c| c.document_id == document_id)
            .map(|c| c.chunk_id.to_string())
            .collect();

        for id in &ids_to_remove {
            self.term_freqs.remove(id);
            self.doc_lengths.remove(id);
        }

        self.chunks.retain(|c| c.document_id != document_id);

        if before != self.chunks.len() {
            self.recompute_stats();
        }

        Ok(before - self.chunks.len())
    }

    fn search(&self, query: &str, limit: usize) -> Result<Vec<SearchResult>, KnowledgeError> {
        if self.chunks.is_empty() {
            return Ok(Vec::new());
        }

        let query_terms = Self::tokenize(query);

        let mut scored: Vec<(f32, usize)> = self
            .chunks
            .iter()
            .enumerate()
            .map(|(idx, chunk)| {
                let score = self.bm25_score(&query_terms, &chunk.chunk_id.to_string());
                (score, idx)
            })
            .collect();

        // 过滤零分，按分数降序排序
        scored.retain(|(score, _)| *score > 0.0);
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);

        Ok(scored
            .into_iter()
            .map(|(score, idx)| {
                let chunk = &self.chunks[idx];
                SearchResult {
                    chunk_id: chunk.chunk_id.to_string(),
                    document_id: chunk.document_id.clone(),
                    chunk_index: chunk.chunk_index,
                    content: chunk.content.clone(),
                    score,
                }
            })
            .collect())
    }

    fn clear(&mut self) -> Result<(), KnowledgeError> {
        self.chunks.clear();
        self.term_freqs.clear();
        self.doc_lengths.clear();
        self.doc_freqs.clear();
        self.avgdl = 0.0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::ChunkId;

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
    fn search_returns_relevant_results() {
        let mut index = InMemoryBm25Index::new();

        index
            .add_chunks(&[
                make_chunk("Rust 是一门系统编程语言，注重安全和性能", "doc1", 0),
                make_chunk("Python 是一门动态语言，广泛用于数据科学", "doc2", 0),
                make_chunk("JavaScript 是网页开发的核心语言", "doc3", 0),
            ])
            .unwrap();

        let results = index.search("Rust 编程", 3).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("Rust"));
    }

    #[test]
    fn remove_document() {
        let mut index = InMemoryBm25Index::new();

        index
            .add_chunks(&[
                make_chunk("Rust content test", "doc1", 0),
                make_chunk("Python content test", "doc2", 0),
            ])
            .unwrap();

        let removed = index.remove_document("doc1").unwrap();
        assert_eq!(removed, 1);

        let results = index.search("Rust", 10).unwrap();
        assert!(results.iter().all(|r| r.document_id != "doc1"));
    }

    #[test]
    fn empty_index_returns_empty() {
        let index = InMemoryBm25Index::new();
        let results = index.search("anything", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn clear_removes_all() {
        let mut index = InMemoryBm25Index::new();

        index
            .add_chunks(&[make_chunk("test content here", "doc1", 0)])
            .unwrap();

        index.clear().unwrap();

        let results = index.search("test", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn chinese_text_search() {
        let mut index = InMemoryBm25Index::new();

        index
            .add_chunks(&[
                make_chunk("ChatPM 是一个本地优先的聊天应用", "doc1", 0),
                make_chunk("Python 是一门流行的编程语言", "doc2", 0),
            ])
            .unwrap();

        let results = index.search("聊天应用", 5).unwrap();
        assert!(!results.is_empty());
        assert!(results[0].content.contains("ChatPM"));
    }
}
