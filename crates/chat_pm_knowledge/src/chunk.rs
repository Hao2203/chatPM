use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 文本块的唯一标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChunkId(Uuid);

impl ChunkId {
    /// 生成一个新的块 ID。
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    /// 获取内部 Uuid。
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl std::fmt::Display for ChunkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for ChunkId {
    fn default() -> Self {
        Self::new()
    }
}

/// 文本块的领域模型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentChunk {
    /// 块的唯一标识。
    pub chunk_id: ChunkId,
    /// 所属知识库 ID。
    pub knowledge_base_id: String,
    /// 逻辑文档标识（文件名或用户给定标题）。
    pub document_id: String,
    /// 块在文档中的位置（从 0 开始）。
    pub chunk_index: usize,
    /// 块文本内容。
    pub content: String,
    /// 字符数（用于过滤）。
    pub char_count: usize,
}

/// 文档分块配置。
#[derive(Debug, Clone)]
pub struct ChunkConfig {
    /// 最大块大小（字符数）。
    pub max_chunk_size: usize,
    /// 相邻块之间的重叠字符数。
    pub chunk_overlap: usize,
    /// 最小块大小（短于此长度的块会被丢弃）。
    pub min_chunk_size: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        Self {
            max_chunk_size: 1024,
            chunk_overlap: 128,
            min_chunk_size: 50,
        }
    }
}

/// 递归字符分割文本为重叠的块。
///
/// 分割策略：
/// 1. 按 `\n\n`（段落）分割
/// 2. 如果某段超过 `max_chunk_size`，按 `\n`（行）分割
/// 3. 如果仍超过，按空格（词）分割
/// 4. 如果仍超过，硬分割在 `max_chunk_size` 处
/// 5. 相邻块之间应用 `chunk_overlap` 字符重叠
/// 6. 丢弃短于 `min_chunk_size` 的块
pub fn chunk_text(text: &str, config: &ChunkConfig) -> Vec<String> {
    if text.trim().is_empty() {
        return vec![];
    }

    // Step 1-4: 递归分割
    let segments = split_recursive(text.trim(), config.max_chunk_size);

    // Step 5: 应用重叠
    let chunks = apply_overlap(&segments, config.chunk_overlap, config.max_chunk_size);

    // Step 6: 过滤太短的块
    chunks
        .into_iter()
        .filter(|c| c.trim().len() >= config.min_chunk_size)
        .collect()
}

/// 递归分割文本段。
fn split_recursive(text: &str, max_size: usize) -> Vec<String> {
    if text.len() <= max_size {
        return vec![text.to_string()];
    }

    // 按段落分割
    let paragraphs = split_by_separator(text, "\n\n");
    if paragraphs.len() > 1 {
        return paragraphs
            .into_iter()
            .flat_map(|p| split_recursive(&p, max_size))
            .collect();
    }

    // 按行分割
    let lines = split_by_separator(text, "\n");
    if lines.len() > 1 {
        return lines
            .into_iter()
            .flat_map(|l| split_recursive(&l, max_size))
            .collect();
    }

    // 按空格分割
    let words = split_by_separator(text, " ");
    if words.len() > 1 {
        return merge_words_to_chunks(&words, max_size);
    }

    // 硬分割：按字符切分
    hard_split(text, max_size)
}

/// 按分隔符分割，保留非空段。
fn split_by_separator(text: &str, sep: &str) -> Vec<String> {
    text.split(sep)
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 将单词列表合并为不超过 max_size 的块。
fn merge_words_to_chunks(words: &[String], max_size: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut current = String::new();

    for word in words {
        if current.is_empty() {
            current = word.clone();
        } else if current.len() + 1 + word.len() <= max_size {
            current.push(' ');
            current.push_str(word);
        } else {
            if !current.is_empty() {
                chunks.push(std::mem::take(&mut current));
            }
            // 如果单个词超过 max_size，硬分割
            if word.len() > max_size {
                chunks.extend(hard_split(word, max_size));
            } else {
                current = word.clone();
            }
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    chunks
}

/// 硬分割：在 max_size 处强制截断。
fn hard_split(text: &str, max_size: usize) -> Vec<String> {
    text.chars()
        .collect::<Vec<_>>()
        .chunks(max_size)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

/// 为相邻块应用重叠。
fn apply_overlap(chunks: &[String], overlap: usize, _max_size: usize) -> Vec<String> {
    if overlap == 0 || chunks.len() <= 1 {
        return chunks.to_vec();
    }

    let mut result: Vec<String> = Vec::with_capacity(chunks.len());
    result.push(chunks[0].clone());

    for i in 1..chunks.len() {
        let prev = &chunks[i - 1];
        let current = &chunks[i];

        // 从前一个块的末尾取 overlap 个字符作为前缀
        if prev.len() > overlap {
            let overlap_text: String = prev.chars().rev().take(overlap).collect::<Vec<_>>().into_iter().rev().collect();
            let merged = format!("{} {}", overlap_text, current);
            result.push(merged);
        } else {
            result.push(current.clone());
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> ChunkConfig {
        ChunkConfig::default()
    }

    // ── 空输入 ──────────────────────────────────────────────────

    #[test]
    fn empty_text_returns_empty() {
        let result = chunk_text("", &default_config());
        assert!(result.is_empty());
    }

    #[test]
    fn whitespace_only_returns_empty() {
        let result = chunk_text("   \n\n  ", &default_config());
        assert!(result.is_empty());
    }

    // ── 短文本（不超过 max_chunk_size） ────────────────────────

    #[test]
    fn short_text_returns_single_chunk() {
        let config = ChunkConfig {
            max_chunk_size: 100,
            chunk_overlap: 0,
            min_chunk_size: 10,
        };
        let result = chunk_text("Hello world", &config);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Hello world");
    }

    // ── 段落分割 ────────────────────────────────────────────────

    #[test]
    fn splits_by_paragraphs() {
        let config = ChunkConfig {
            max_chunk_size: 50,
            chunk_overlap: 0,
            min_chunk_size: 5,
        };
        let text = "第一段内容。\n\n第二段内容。\n\n第三段内容。";
        let result = chunk_text(text, &config);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "第一段内容。");
        assert_eq!(result[1], "第二段内容。");
        assert_eq!(result[2], "第三段内容。");
    }

    // ── 行分割 ──────────────────────────────────────────────────

    #[test]
    fn splits_by_lines_when_paragraph_too_long() {
        let config = ChunkConfig {
            max_chunk_size: 10,
            chunk_overlap: 0,
            min_chunk_size: 3,
        };
        let text = "第一行\n第二行\n第三行";
        let result = chunk_text(text, &config);
        // 每行都短于 10 字符，所以按行分割
        assert!(result.len() >= 3);
    }

    // ── 重叠 ────────────────────────────────────────────────────

    #[test]
    fn applies_overlap_between_chunks() {
        let config = ChunkConfig {
            max_chunk_size: 20,
            chunk_overlap: 5,
            min_chunk_size: 10,
        };
        let text = "这是第一段很长的内容。这是第二段很长的内容。这是第三段很长的内容。这是第四段。";
        let result = chunk_text(text, &config);
        // 验证至少产生多个块
        assert!(result.len() > 1, "长文本应产生多个块，实际: {:?}", result);
    }

    // ── 最小块过滤 ──────────────────────────────────────────────

    #[test]
    fn filters_short_chunks() {
        let config = ChunkConfig {
            max_chunk_size: 1000,
            chunk_overlap: 0,
            min_chunk_size: 100,
        };
        let text = "短。";
        let result = chunk_text(text, &config);
        assert!(result.is_empty(), "短于 min_chunk_size 的块应被过滤");
    }

    // ── 中英文混合 ──────────────────────────────────────────────

    #[test]
    fn handles_chinese_text() {
        let config = ChunkConfig {
            max_chunk_size: 200,
            chunk_overlap: 20,
            min_chunk_size: 20,
        };
        let text = "ChatPM 是一个本地优先的聊天应用，未来将支持端到端加密同步。\
                     所有聊天记录本地存储在 SQLite 中。\
                     技术栈为 Rust workspace（核心逻辑 + Tauri 后端）\
                     + Tauri 2.x（桌面壳）+ SvelteKit 5（UI，SPA 模式）。";
        let result = chunk_text(text, &config);
        assert!(!result.is_empty(), "中文文本应能正常分块");
    }

    // ── 长段落硬分割 ────────────────────────────────────────────

    #[test]
    fn hard_splits_very_long_word() {
        let config = ChunkConfig {
            max_chunk_size: 5,
            chunk_overlap: 0,
            min_chunk_size: 3,
        };
        let text = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let result = chunk_text(text, &config);
        // 每个块应该不超过 max_chunk_size（硬分割后）
        for chunk in &result {
            assert!(chunk.len() <= 5, "块 '{}' 长度 {}", chunk, chunk.len());
        }
        assert!(result.len() > 1);
    }
}
