use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use chat_pm_session::{chat::TurnId, memory::Memory, session::SessionId};
use uuid::Uuid;

// ── Schema ──────────────────────────────────────────────────────────

const SCHEMA_SQL: &str = "
CREATE TABLE IF NOT EXISTS sessions (
    session_id  TEXT PRIMARY KEY,
    created_at  INTEGER NOT NULL,
    title        TEXT,
    user_persona TEXT
);

CREATE TABLE IF NOT EXISTS turns (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id   TEXT NOT NULL,
    turn_num     INTEGER NOT NULL,
    user_text    TEXT NOT NULL,
    assistant_text TEXT NOT NULL,
    created_at   INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id),
    UNIQUE(session_id, turn_num)
);

CREATE TABLE IF NOT EXISTS config (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS summaries (
    session_id    TEXT PRIMARY KEY,
    content       TEXT NOT NULL,
    last_turn_num INTEGER NOT NULL,
    created_at    INTEGER NOT NULL,
    FOREIGN KEY (session_id) REFERENCES sessions(session_id)
);
";

const MIGRATE_V1_SQL: &str = "
ALTER TABLE turns ADD COLUMN prompt_tokens INTEGER;
ALTER TABLE turns ADD COLUMN completion_tokens INTEGER;
";

// ── Domain records ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
    pub title: Option<String>,
    pub user_persona: Option<String>,
}

#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub user_text: String,
    pub assistant_text: String,
    pub created_at: DateTime<Utc>,
    pub prompt_tokens: Option<i64>,
    pub completion_tokens: Option<i64>,
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
pub struct ChatDb {
    conn: Arc<Mutex<Connection>>,
}

impl ChatDb {
    /// 获取数据库连接锁（内部辅助方法）。
    fn lock_conn(&self) -> DbResult<std::sync::MutexGuard<'_, Connection>> {
        self.conn.lock().map_err(|_| DbError::Lock)
    }

    /// 打开（或创建）一个持久化的 SQLite 数据库文件。
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA_SQL)?;
        // Run migration (ignore errors if columns already exist)
        let _ = conn.execute_batch(MIGRATE_V1_SQL);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// 打开一个内存数据库（主要用于测试）。
    pub fn open_in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA_SQL)?;
        let _ = conn.execute_batch(MIGRATE_V1_SQL);
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    // ── Session ─────────────────────────────────────────────────

    pub fn create_session(&self, session_id: SessionId) -> DbResult<()> {
        self.upsert_session(SessionRecord {
            session_id,
            created_at: Utc::now(),
            title: None,
            user_persona: None,
        })
    }

    pub fn session_exists(&self, session_id: SessionId) -> DbResult<bool> {
        Ok(self.get_session(session_id)?.is_some())
    }

    pub fn delete_session(&self, session_id: SessionId) -> DbResult<bool> {
        let conn = self.lock_conn()?;
        conn.execute_batch("BEGIN")?;
        conn.execute(
            "DELETE FROM turns WHERE session_id = ?1",
            params![session_id.as_uuid()],
        )?;
        conn.execute(
            "DELETE FROM summaries WHERE session_id = ?1",
            params![session_id.as_uuid()],
        )?;
        let rows = conn.execute(
            "DELETE FROM sessions WHERE session_id = ?1",
            params![session_id.as_uuid()],
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
                record.session_id.as_uuid(),
                record.created_at.timestamp(),
                record.title,
                record.user_persona,
            ],
        )?;
        Ok(())
    }

    pub fn get_session(&self, session_id: SessionId) -> DbResult<Option<SessionRecord>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT session_id, created_at, title, user_persona FROM sessions WHERE session_id = ?1",
                params![session_id.as_uuid()],
                |row| {
                    let created_at: i64 = row.get(1)?;
                    let sid: Uuid = row.get(0)?;
                    Ok(SessionRecord {
                        session_id: SessionId::from_uuid(sid),
                        created_at: from_sql_timestamp(created_at)?,
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
            let created_at: i64 = row.get(1)?;
            let sid: Uuid = row.get(0)?;
            Ok(SessionRecord {
                session_id: SessionId::from_uuid(sid),
                created_at: from_sql_timestamp(created_at)?,
                title: row.get(2)?,
                user_persona: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_session_title(&self, session_id: SessionId, title: &str) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "UPDATE sessions SET title = ?1 WHERE session_id = ?2",
            params![title, session_id.as_uuid()],
        )?;
        Ok(())
    }

    pub fn get_session_title(&self, session_id: SessionId) -> DbResult<Option<String>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT title FROM sessions WHERE session_id = ?1",
                params![session_id.as_uuid()],
                |row| row.get(0),
            )
            .optional()?)
    }

    // ── Turns ───────────────────────────────────────────────────

    pub fn append_chat_turn(
        &self,
        session_id: SessionId,
        user_text: String,
        assistant_text: String,
        prompt_tokens: Option<i64>,
        completion_tokens: Option<i64>,
    ) -> DbResult<()> {
        let turn_id = self.next_turn_id(session_id)?;
        self.append_turn(TurnRecord {
            turn_id,
            session_id,
            user_text,
            assistant_text,
            created_at: Utc::now(),
            prompt_tokens,
            completion_tokens,
        })
    }

    pub fn append_turn(&self, record: TurnRecord) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO turns (session_id, turn_num, user_text, assistant_text, created_at, prompt_tokens, completion_tokens)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                record.session_id.as_uuid(),
                record.turn_id.get(),
                record.user_text,
                record.assistant_text,
                record.created_at.timestamp(),
                record.prompt_tokens,
                record.completion_tokens,
            ],
        )?;
        Ok(())
    }

    pub fn recent_turns(&self, session_id: SessionId, n: usize) -> DbResult<Vec<TurnRecord>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_num, session_id, user_text, assistant_text, created_at, prompt_tokens, completion_tokens
             FROM turns
             WHERE session_id = ?1
             ORDER BY turn_num DESC
             LIMIT ?2",
        )?;

        let mut rows: Vec<TurnRecord> = stmt
            .query_map(params![session_id.as_uuid(), n as i64], |row| {
                let created_at: i64 = row.get(4)?;
                let sid: Uuid = row.get(1)?;
                Ok(TurnRecord {
                    turn_id: TurnId::new(row.get::<_, i64>(0)? as u64),
                    session_id: SessionId::from_uuid(sid),
                    user_text: row.get(2)?,
                    assistant_text: row.get(3)?,
                    created_at: from_sql_timestamp(created_at)?,
                    prompt_tokens: row.get(5)?,
                    completion_tokens: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        rows.reverse();
        Ok(rows)
    }

    pub fn load_recent_memory(&self, session_id: SessionId, n: usize) -> DbResult<Vec<Memory>> {
        Ok(self
            .recent_turns(session_id, n)?
            .into_iter()
            .map(|t| t.to_memory_chunk())
            .collect())
    }

    pub fn next_turn_id(&self, session_id: SessionId) -> DbResult<TurnId> {
        let conn = self.lock_conn()?;
        let max: i64 = conn.query_row(
            "SELECT COALESCE(MAX(turn_num), 0) FROM turns WHERE session_id = ?1",
            params![session_id.as_uuid()],
            |row| row.get(0),
        )?;
        Ok(TurnId::new(max as u64 + 1))
    }

    // ── Summaries ───────────────────────────────────────────────

    /// 插入或更新会话摘要。
    pub fn upsert_summary(
        &self,
        session_id: SessionId,
        content: &str,
        last_turn_num: i64,
    ) -> DbResult<()> {
        let conn = self.lock_conn()?;
        conn.execute(
            "INSERT INTO summaries (session_id, content, last_turn_num, created_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                 content = excluded.content,
                 last_turn_num = excluded.last_turn_num,
                 created_at = excluded.created_at",
            params![
                session_id.as_uuid(),
                content,
                last_turn_num,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    /// 读取会话摘要，返回 `(content, last_turn_num)`。
    pub fn get_summary(&self, session_id: SessionId) -> DbResult<Option<(String, i64)>> {
        let conn = self.lock_conn()?;
        Ok(conn
            .query_row(
                "SELECT content, last_turn_num FROM summaries WHERE session_id = ?1",
                params![session_id.as_uuid()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// 统计会话的总轮次数。
    pub fn count_turns(&self, session_id: SessionId) -> DbResult<u64> {
        let conn = self.lock_conn()?;
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM turns WHERE session_id = ?1",
            params![session_id.as_uuid()],
            |row| row.get(0),
        )?;
        Ok(count as u64)
    }

    /// 按 turn_num 范围读取轮次（包含两端），按时间顺序返回。
    pub fn get_turns_range(
        &self,
        session_id: SessionId,
        from: u64,
        to: u64,
    ) -> DbResult<Vec<Memory>> {
        let conn = self.lock_conn()?;
        let mut stmt = conn.prepare(
            "SELECT turn_num, user_text, assistant_text
             FROM turns
             WHERE session_id = ?1 AND turn_num BETWEEN ?2 AND ?3
             ORDER BY turn_num ASC",
        )?;

        let rows = stmt
            .query_map(
                params![session_id.as_uuid(), from as i64, to as i64],
                |row| {
                    Ok(Memory {
                        user_text: row.get(1)?,
                        assistant_text: row.get(2)?,
                    })
                },
            )?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(rows)
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
    pub fn to_memory_chunk(self) -> Memory {
        Memory {
            user_text: self.user_text,
            assistant_text: self.assistant_text,
        }
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// 从 SQLite INTEGER 时间戳（秒）解析 `DateTime<Utc>`（用于 in-row-closure 场景）。
fn from_sql_timestamp(secs: i64) -> Result<DateTime<Utc>, rusqlite::Error> {
    DateTime::from_timestamp(secs, 0).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            0,
            rusqlite::types::Type::Integer,
            Box::new(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("invalid unix timestamp: {secs}"),
            )),
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
