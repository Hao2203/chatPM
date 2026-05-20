use crate::{message::UserInput, prompt::TitlePrompt};
use uuid::Uuid;

// ── Session identity ─────────────────────────────────────────────────

/// 会话唯一标识符。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SessionId(Uuid);

impl SessionId {
    /// 生成新的 v7 UUID 作为会话 ID。
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }

    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Session title ────────────────────────────────────────────────────

/// 会话标题。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Title(String);

impl Title {
    pub fn new(title: String) -> Self {
        Self(title)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

impl std::fmt::Display for Title {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ── Session lifecycle states ─────────────────────────────────────────

/// 新创建的会话：尚无标题，无法进行对话。
///
/// 生命周期：`NewSession` → [`TitlePrompt`] → [`Session`]
#[derive(Debug, Clone)]
pub struct NewSession {
    session_id: SessionId,
}

impl NewSession {
    /// 用已有 `SessionId` 构造 `NewSession`。
    pub fn with_id(session_id: SessionId) -> Self {
        Self { session_id }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// 转入标题生成阶段，消耗自身。
    ///
    /// `NewSession` → `TitlePrompt`
    pub fn into_title_prompt(self, user_input: &UserInput) -> TitlePrompt<'_> {
        TitlePrompt::new(self.session_id, user_input.as_str())
    }
}

/// 已完成标题生成的正式会话：可以在此之上进行对话。
#[derive(Debug, Clone)]
pub struct Session {
    session_id: SessionId,
    title: Title,
}

impl Session {
    /// 从持久化记录恢复（已确认 title 存在）。
    pub fn resume(session_id: SessionId, title: Title) -> Self {
        Self { session_id, title }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    pub fn title(&self) -> &Title {
        &self.title
    }
}
