use chat_pm_commands::session::PipelineError;
use chat_pm_database::DbError;
use chat_pm_deepseek::ApiError;
use chat_pm_session::ChatError;
use serde::Serialize;

/// 统一的 Tauri 命令错误类型。
///
/// 序列化为 `{ kind: string, message: string }`，前端可根据 `kind` 做差异化处理。
#[derive(Debug, Clone, Serialize)]
pub struct AppError {
    /// 错误类别：`"db"` | `"api"` | `"validation"` | `"locked"` | `"internal"`
    pub kind: String,
    /// 面向用户的错误描述
    pub message: String,
}

impl AppError {
    pub fn new(kind: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            message: message.into(),
        }
    }

    pub fn locked() -> Self {
        Self::new("locked", "资源暂不可用，请稍后重试")
    }

    pub fn not_configured() -> Self {
        Self::new("validation", "请先配置 API Key")
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)
    }
}

// ── From 转换：下层错误类型 → AppError ────────────────────────────

impl From<ChatError> for AppError {
    fn from(e: ChatError) -> Self {
        Self::new("validation", e.to_string())
    }
}

impl From<DbError> for AppError {
    fn from(e: DbError) -> Self {
        Self::new("db", e.to_string())
    }
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        Self::new("api", e.to_string())
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self::new("internal", e.to_string())
    }
}

impl From<PipelineError> for AppError {
    fn from(e: PipelineError) -> Self {
        match e {
            PipelineError::Chat(d) => d.into(),
            PipelineError::Db(d) => d.into(),
            PipelineError::Api(a) => a.into(),
            PipelineError::Internal(a) => a.into(),
        }
    }
}
