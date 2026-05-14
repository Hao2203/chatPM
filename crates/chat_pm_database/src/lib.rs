/// # 内存数据库
///
/// 用 `HashMap` + `Vec` 模拟两张表：
///   - `sessions`  : session_id → 会话元数据
///   - `turns`     : session_id → Vec<TurnRecord>（按轮次追加）
///
/// 向量相似度检索用纯 Rust 余弦距离实现，无外部依赖。
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use chat_pm_conversation::chat::{MemoryChunk, Similarity, TurnId};

// ─────────────────────────────────────────
// 数据记录定义
// ─────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub created_at: DateTime<Utc>,
    pub user_persona: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub turn_id: TurnId,
    pub session_id: String,
    pub user_text: String,
    pub assistant_text: String,
    /// Embedding 向量（存 f32 数组）
    pub embedding: Vec<f32>,
    pub created_at: DateTime<Utc>,
}

// ─────────────────────────────────────────
// 内存数据库结构
// ─────────────────────────────────────────

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionRecord>,
    /// session_id → 按时间顺序排列的 TurnRecord
    turns: HashMap<String, Vec<TurnRecord>>,
}

/// 线程安全的内存数据库（`Arc<RwLock<...>>`）
#[derive(Debug, Clone, Default)]
pub struct MemoryDb {
    inner: Arc<RwLock<Inner>>,
}

impl MemoryDb {
    pub fn new() -> Self {
        Self::default()
    }

    // ── 会话操作 ──────────────────────────

    /// 创建或更新会话
    pub fn upsert_session(&self, record: SessionRecord) {
        let mut db = self.inner.write().unwrap();
        db.sessions.insert(record.session_id.clone(), record);
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionRecord> {
        self.inner.read().unwrap().sessions.get(session_id).cloned()
    }

    // ── 轮次操作 ──────────────────────────

    /// 追加一轮对话记录
    pub fn append_turn(&self, record: TurnRecord) {
        let mut db = self.inner.write().unwrap();
        db.turns
            .entry(record.session_id.clone())
            .or_default()
            .push(record);
    }

    /// 取最近 `n` 轮对话（短期记忆）
    pub fn recent_turns(&self, session_id: &str, n: usize) -> Vec<TurnRecord> {
        let db = self.inner.read().unwrap();
        let turns = db
            .turns
            .get(session_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);
        let start = turns.len().saturating_sub(n);
        turns[start..].to_vec()
    }

    /// 语义检索：用余弦相似度找 Top-K 最相关轮次（长期记忆）
    ///
    /// 排除 `exclude_turn_ids` 中的轮次，避免短期记忆重复进入长期记忆。
    pub fn semantic_search(
        &self,
        session_id: &str,
        query_vec: &[f32],
        top_k: usize,
        exclude_turn_ids: &[TurnId],
    ) -> Vec<(TurnRecord, Similarity)> {
        let db = self.inner.read().unwrap();
        let turns = match db.turns.get(session_id) {
            Some(v) => v,
            None => return vec![],
        };

        let mut scored: Vec<(TurnRecord, Similarity)> = turns
            .iter()
            .filter(|t| !exclude_turn_ids.contains(&t.turn_id))
            .filter(|t| !t.embedding.is_empty())
            .map(|t| {
                let sim = cosine_similarity(query_vec, &t.embedding);
                (t.clone(), Similarity(sim))
            })
            .collect();

        // 按相似度降序排列
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(top_k);
        scored
    }

    /// 返回当前会话的下一个 TurnId（自增）
    pub fn next_turn_id(&self, session_id: &str) -> TurnId {
        let db = self.inner.read().unwrap();
        let len = db.turns.get(session_id).map(|v| v.len()).unwrap_or(0);
        TurnId(len as u64 + 1)
    }

    /// 统计信息（调试用）
    pub fn stats(&self) -> DbStats {
        let db = self.inner.read().unwrap();
        let total_turns: usize = db.turns.values().map(|v| v.len()).sum();
        DbStats {
            session_count: db.sessions.len(),
            total_turn_count: total_turns,
        }
    }
}

#[derive(Debug)]
pub struct DbStats {
    pub session_count: usize,
    pub total_turn_count: usize,
}

// ─────────────────────────────────────────
// TurnRecord → MemoryChunk
// ─────────────────────────────────────────

impl TurnRecord {
    pub fn to_memory_chunk(&self, relevance: Similarity) -> MemoryChunk {
        MemoryChunk {
            turn: self.turn_id,
            user_text: self.user_text.clone(),
            assistant_text: self.assistant_text.clone(),
            relevance,
        }
    }
}

// ─────────────────────────────────────────
// 工具函数
// ─────────────────────────────────────────

/// 余弦相似度，向量长度不同时截断到较短者
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    let len = a.len().min(b.len());
    if len == 0 {
        return 0.0;
    }
    let dot: f32 = a[..len]
        .iter()
        .zip(b[..len].iter())
        .map(|(x, y)| x * y)
        .sum();
    let norm_a: f32 = a[..len].iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b[..len].iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        0.0
    } else {
        (dot / (norm_a * norm_b)).clamp(-1.0, 1.0)
    }
}
