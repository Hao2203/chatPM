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

use chat_pm_session::{chat::TurnId, memory::Memory};

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
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Default)]
struct Inner {
    sessions: HashMap<String, SessionRecord>,
    turns: HashMap<String, Vec<TurnRecord>>,
}

#[derive(Debug, Clone, Default)]
pub struct MemoryDb {
    inner: Arc<RwLock<Inner>>,
}

impl MemoryDb {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create_session(&self, session_id: &str) {
        self.upsert_session(SessionRecord {
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            user_persona: None,
        });
    }

    pub fn session_exists(&self, session_id: &str) -> bool {
        self.get_session(session_id).is_some()
    }

    pub fn load_recent_memory(&self, session_id: &str, n: usize) -> Vec<Memory> {
        self.recent_turns(session_id, n)
            .into_iter()
            .map(|x| x.to_memory_chunk())
            .collect()
    }

    pub fn append_chat_turn(&self, session_id: &str, user_text: String, assistant_text: String) {
        let turn_id = self.next_turn_id(session_id);
        self.append_turn(TurnRecord {
            turn_id,
            session_id: session_id.to_string(),
            user_text,
            assistant_text,
            created_at: Utc::now(),
        });
    }

    pub fn upsert_session(&self, record: SessionRecord) {
        let mut db = self.inner.write().unwrap();
        db.sessions.insert(record.session_id.clone(), record);
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionRecord> {
        self.inner.read().unwrap().sessions.get(session_id).cloned()
    }

    pub fn append_turn(&self, record: TurnRecord) {
        let mut db = self.inner.write().unwrap();
        db.turns
            .entry(record.session_id.clone())
            .or_default()
            .push(record);
    }

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

    pub fn next_turn_id(&self, session_id: &str) -> TurnId {
        let db = self.inner.read().unwrap();
        let len = db.turns.get(session_id).map(|v| v.len()).unwrap_or(0);
        TurnId(len as u64 + 1)
    }

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

impl TurnRecord {
    pub fn to_memory_chunk(&self) -> Memory {
        Memory {
            user_text: self.user_text.clone(),
            assistant_text: self.assistant_text.clone(),
        }
    }
}

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
