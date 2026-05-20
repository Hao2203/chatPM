use chat_pm_commands::session::CommandError;
use chat_pm_database::DbError;
use chat_pm_deepseek::ApiError;
use chat_pm_session::ChatError;
use serde::Serialize;

/// AppError 的错误类别。
#[derive(Debug, Clone, Serialize, PartialEq)]
pub enum ErrorKind {
    #[serde(rename = "db")]
    Db,
    #[serde(rename = "api")]
    Api,
    #[serde(rename = "validation")]
    Validation,
    #[serde(rename = "locked")]
    Locked,
    #[serde(rename = "internal")]
    Internal,
}

impl std::fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Db => write!(f, "db"),
            Self::Api => write!(f, "api"),
            Self::Validation => write!(f, "validation"),
            Self::Locked => write!(f, "locked"),
            Self::Internal => write!(f, "internal"),
        }
    }
}

/// 统一的 Tauri 命令错误类型。
///
/// 序列化为 `{ kind, message }` 供前端消费，`source` 保留原始错误链。
#[derive(Debug, Serialize)]
pub struct AppError {
    pub kind: ErrorKind,
    /// 面向用户的错误描述，默认使用 source 的 Display 输出。
    pub message: String,
    /// 原始错误（序列化时跳过）。
    #[serde(skip)]
    pub source: Option<anyhow::Error>,
}

impl AppError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub fn locked() -> Self {
        Self::new(ErrorKind::Locked, "资源暂不可用，请稍后重试")
    }

    pub fn not_configured() -> Self {
        Self::new(ErrorKind::Validation, "请先配置 API Key")
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}] {}", self.kind, self.message)
    }
}

impl std::error::Error for AppError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_ref().map(|e| e.as_ref())
    }
}

// ── From 转换：下层错误类型 → AppError ────────────────────────────

impl From<ChatError> for AppError {
    fn from(e: ChatError) -> Self {
        Self {
            kind: ErrorKind::Validation,
            message: e.to_string(),
            source: Some(e.into()),
        }
    }
}

impl From<DbError> for AppError {
    fn from(e: DbError) -> Self {
        Self {
            kind: ErrorKind::Db,
            message: e.to_string(),
            source: Some(e.into()),
        }
    }
}

impl From<ApiError> for AppError {
    fn from(e: ApiError) -> Self {
        Self {
            kind: ErrorKind::Api,
            message: e.to_string(),
            source: Some(e.into()),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        Self {
            kind: ErrorKind::Internal,
            message: e.to_string(),
            source: Some(e),
        }
    }
}

impl From<CommandError> for AppError {
    fn from(e: CommandError) -> Self {
        match e {
            CommandError::Chat(d) => d.into(),
            CommandError::Db(d) => d.into(),
            CommandError::Api(a) => a.into(),
            CommandError::Internal(a) => a.into(),
        }
    }
}
