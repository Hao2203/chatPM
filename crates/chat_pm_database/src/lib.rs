use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use chat_pm_session::{chat::TurnId, memory::Memory};

// ── Schema ──────────────────────────────────────────────────────────

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id  TEXT PRIMARY KEY,
    created_at  TEXT NOT NULL,
    user_persona TEXT
);

CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    turn_num     INTEGER NOT NULL,
    user_text    TEXT NOT NULL,
    assistant_text TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id),
    UNIQUE(session_id, turn_num)
);

CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

// ── Domain records ──────────────────────────────────────────────────

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

#[derive(Debug)]
pub struct DbStats {
    pub session_count: usize,
    pub total_turn_count: usize,
}

// ── Database handle ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MemoryDb {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryDb {
    /// 打开（或创建）一个持久化的 SQLite 数据库文件。
    pub fn open(path: impl AsRef<Path>) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开一个内存数据库（主要用于测试）。
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Session ─────────────────────────────────────────────────

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

    pub fn upsert_session(&self, record: SessionRecord) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO sessions (session_id, created_at, user_persona)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET
                 created_at   = excluded.created_at,
                 user_persona = excluded.user_persona",
            params![
                record.session_id,
                record.created_at.to_rfc3339(),
                record.user_persona,
            ],
        )
        .unwrap();
    }

    pub fn get_session(&self, session_id: &str) -> Option<SessionRecord> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT session_id, created_at, user_persona FROM sessions WHERE session_id = ?1",
            params![session_id],
            |row| {
                let created_at: String = row.get(1)?;
                Ok(SessionRecord {
                    session_id: row.get(0)?,
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .unwrap()
                        .with_timezone(&Utc),
                    user_persona: row.get(2)?,
                })
            },
        )
        .ok()
    }

    pub fn list_sessions(&self) -> Vec<SessionRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT session_id, created_at, user_persona FROM sessions ORDER BY created_at DESC",
            )
            .unwrap();
        stmt.query_map([], |row| {
            let created_at: String = row.get(1)?;
            Ok(SessionRecord {
                session_id: row.get(0)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .unwrap()
                    .with_timezone(&Utc),
                user_persona: row.get(2)?,
            })
        })
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
    }

    // ── Turns ───────────────────────────────────────────────────

    pub fn append_chat_turn(
        &self,
        session_id: &str,
        user_text: String,
        assistant_text: String,
    ) {
        let turn_id = self.next_turn_id(session_id);
        self.append_turn(TurnRecord {
            turn_id,
            session_id: session_id.to_string(),
            user_text,
            assistant_text,
            created_at: Utc::now(),
        });
    }

    pub fn append_turn(&self, record: TurnRecord) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO turns (session_id, turn_num, user_text, assistant_text, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                record.session_id,
                record.turn_id.0,
                record.user_text,
                record.assistant_text,
                record.created_at.to_rfc3339(),
            ],
        )
        .unwrap();
    }

    pub fn recent_turns(&self, session_id: &str, n: usize) -> Vec<TurnRecord> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT turn_num, session_id, user_text, assistant_text, created_at
                 FROM turns
                 WHERE session_id = ?1
                 ORDER BY turn_num DESC
                 LIMIT ?2",
            )
            .unwrap();

        let mut rows: Vec<TurnRecord> = stmt
            .query_map(params![session_id, n as i64], |row| {
                let created_at: String = row.get(4)?;
                Ok(TurnRecord {
                    turn_id: TurnId(row.get::<_, i64>(0)? as u64),
                    session_id: row.get(1)?,
                    user_text: row.get(2)?,
                    assistant_text: row.get(3)?,
                    created_at: DateTime::parse_from_rfc3339(&created_at)
                        .unwrap()
                        .with_timezone(&Utc),
                })
            })
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        // Reverse to chronological order (oldest first)
        rows.reverse();
        rows
    }

    pub fn load_recent_memory(&self, session_id: &str, n: usize) -> Vec<Memory> {
        self.recent_turns(session_id, n)
            .into_iter()
            .map(|t| t.to_memory_chunk())
            .collect()
    }

    pub fn next_turn_id(&self, session_id: &str) -> TurnId {
        let conn = self.conn.lock().unwrap();
        let max: i64 = conn
            .query_row(
                "SELECT COALESCE(MAX(turn_num), 0) FROM turns WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .unwrap();
        TurnId(max as u64 + 1)
    }

    // ── Config ──────────────────────────────────────────────────

    pub fn set_config(&self, key: &str, value: &str) {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )
        .unwrap();
    }

    pub fn get_config(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT value FROM config WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .ok()
    }

    // ── Stats ───────────────────────────────────────────────────

    pub fn stats(&self) -> DbStats {
        let conn = self.conn.lock().unwrap();
        let session_count: usize = conn
            .query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))
            .unwrap();
        let total_turn_count: usize = conn
            .query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))
            .unwrap();
        DbStats {
            session_count,
            total_turn_count,
        }
    }
}

// ── TurnRecord → Memory ─────────────────────────────────────────────

impl TurnRecord {
    pub fn to_memory_chunk(&self) -> Memory {
        Memory {
            user_text: self.user_text.clone(),
            assistant_text: self.assistant_text.clone(),
        }
    }
}

// ── Cosine similarity (向量检索，为未来 RAG 准备) ────────────────────

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
