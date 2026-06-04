use chat_pm_session::session::SessionId;
use serde::Serialize;

pub(crate) const SUPPORTED_MODELS: &[&str] = &["deepseek-v4-flash", "deepseek-v4-pro"];
pub(crate) const DEFAULT_MODEL: &str = "deepseek-v4-flash";

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatChunkPayload {
    pub(crate) session_id: SessionId,
    pub(crate) content: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ChatDonePayload {
    pub(crate) session_id: SessionId,
    pub(crate) prompt_tokens: Option<usize>,
    pub(crate) completion_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SessionTitlePayload {
    pub(crate) session_id: SessionId,
    pub(crate) title: String,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SessionDeletedPayload {
    pub(crate) session_id: SessionId,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SessionInfo {
    pub(crate) session_id: SessionId,
    pub(crate) created_at: String,
    pub(crate) title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TurnInfo {
    pub(crate) turn_uuid: String,
    pub(crate) turn_num: u64,
    pub(crate) user_text: String,
    pub(crate) assistant_text: String,
    pub(crate) prompt_tokens: Option<i64>,
    pub(crate) completion_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct SyncStatusPayload {
    pub(crate) status: String,
    pub(crate) active: bool,
    pub(crate) ticket: Option<String>,
}
