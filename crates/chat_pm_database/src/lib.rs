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
    title        TEXT,
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
    pub title: Option<String>,
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

// ── Error type ─────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("数据库锁已污染")]
    Lock,
    #[error("SQL 错误: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("日期解析失败: {0}")]
    DateParse(String),
}

pub type DbResult<T> = Result<T, DbError>;

// ── Database handle ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MemoryDb {
    conn: Arc<Mutex<Connection>>,
}

impl MemoryDb {
    /// 获取数据库连接锁（内部辅助方法）。
    fn lock_conn(&self) -> DbResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| DbError::Lock)
    }

    /// 打开（或创建）一个持久化的 SQLite 数据库文件。
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开一个内存数据库（主要用于测试）。
    pub fn open_in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Session ─────────────────────────────────────────────────

    pub fn create_session(&self, session_id: &str) -> DbResult<()> {
        self.upsert_session(SessionRecord {
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            title: None,
            user_persona: None,
        })
    }

    pub fn session_exists(&self, session_id: &str) -> DbResult<bool> {
        Ok(self.get_session(session_id)?.is_some())
    }

    pub fn delete_session(&self, session_id: &str) -> DbResult<bool> {
        let conn = self.lock_conn()?;
        conn.execute_batch("BEGIN")?;
        conn.execute(
            "DELETE FROM turns WHERE session_id = ?1",
            params![session_id],
        )?;
        let rows = conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id],
        )?;
        conn.execute_batch("COMMIT")?;
        Ok(rows > 0)
    }

    pub fn upsert_session(&self, record: SessionRecord) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO sessions (session_id, created_at, title, user_persona)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                 created_at   = excluded.created_at,
                 title        = excluded.title,
                 user_persona = excluded.user_persona",
            params![
                record.session_id,
                record.created_at.to_rfc3339(),
                record.title,
                record.user_persona,
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, session_id: &str) -> DbResult<Option<SessionRecord>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT session_id, created_at, title, user_persona FROM sessions WHERE session_id = ?1",
                params![session_id],
                |row| {
                    let created_at: String = row.get(1)?;
                    Ok(SessionRecord {
                        session_id: row.get(0)?,
                        created_at: parse_rfc3339(&created_at)?,
                        title: row.get(2)?,
                        user_persona: row.get(3)?,
                    })
                },
            )
            .optional()?)
    }

    pub fn list_sessions(&self) -> DbResult<Vec<SessionRecord>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT session_id, created_at, title, user_persona FROM sessions ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(1)?;
            Ok(SessionRecord {
                session_id: row.get(0)?,
                created_at: parse_rfc3339(&created_at)?,
                title: row.get(2)?,
                user_persona: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_session_title(&self, session_id: &str, title: &str) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE session_id = ?2",
            params![title, session_id],
        )?;
        Ok(())
    }

    pub fn get_session_title(&self, session_id: &str) -> DbResult<Option<String>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT title FROM sessions WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    // ── Turns ───────────────────────────────────────────────────

    pub fn append_chat_turn(
        &self,
        session_id: &str,
        user_text: String,
        assistant_text: String,
    ) -> DbResult<()> {
        let turn_id = self.next_turn_id(session_id)?;
        self.append_turn(TurnRecord {
            turn_id,
            session_id: session_id.to_string(),
            user_text,
            assistant_text,
            created_at: Utc::now(),
        })
    }

    pub fn append_turn(&self, record: TurnRecord) -> DbResult<()> {
        let conn = self.lock_conn()?;
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
        )?;
        Ok(())
    }

    pub fn recent_turns(&self, session_id: &str, n: usize) -> DbResult<Vec<TurnRecord>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_num, session_id, user_text, assistant_text, created_at
             FROM turns
             WHERE session_id = ?1
             ORDER BY turn_num DESC
             LIMIT ?2",
        )?;

        let mut rows: Vec<TurnRecord> = stmt
            .query_map(params![session_id, n as i64], |row| {
                let created_at: String = row.get(4)?;
                Ok(TurnRecord {
                    turn_id: TurnId(row.get::<_, i64>(0)? as u64),
                    session_id: row.get(1)?,
                    user_text: row.get(2)?,
                    assistant_text: row.get(3)?,
                    created_at: parse_rfc3339(&created_at)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.reverse();
        Ok(rows)
    }

    pub fn load_recent_memory(&self, session_id: &str, n: usize) -> DbResult<Vec<Memory>> {
        Ok(self
            .recent_turns(session_id, n)?
            .into_iter()
            .map(|t| t.to_memory_chunk())
            .collect())
    }

    pub fn next_turn_id(&self, session_id: &str) -> DbResult<TurnId> {
        let conn = self.lock_conn()?;
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(turn_num), 0) FROM turns WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )?;
        Ok(TurnId(max as u64 + 1))
    }

    // ── Config ──────────────────────────────────────────────────

    pub fn set_config(&self, key: &str, value: &str) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO config (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn get_config(&self, key: &str) -> DbResult<Option<String>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT value FROM config WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    // ── Stats ───────────────────────────────────────────────────

    pub fn stats(&self) -> DbResult<DbStats> {
        let conn = self.lock_conn()?;
        let session_count: usize =
            conn.query_row("SELECT COUNT(*) FROM sessions", [], |row| row.get(0))?;
        let total_turn_count: usize =
            conn.query_row("SELECT COUNT(*) FROM turns", [], |row| row.get(0))?;
        Ok(DbStats {
            session_count,
            total_turn_count,
        })
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

// ── Helpers ──────────────────────────────────────────────────────────

/// 解析 RFC 3339 日期字符串（用于 in-row-closure 场景，返回 `rusqlite::Error`）。
fn parse_rfc3339(s: &str) -> Result<DateTime<Utc>, rusqlite::Error> {
    DateTime::parse_from_rfc3339(s)
        .map(|ts| ts.with_timezone(&Utc))
        .map_err(|e| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Text,
                Box::new(e),
            )
        })
}

/// 将 `QueryReturnedNoRows` 转换为 `None`，其他错误正常传播。
trait OptionalExt<T> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for Result<T, rusqlite::Error> {
    fn optional(self) -> Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
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
