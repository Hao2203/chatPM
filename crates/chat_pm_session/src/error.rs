/// 聊天业务错误：表示违反了聊天应用的核心业务规则。
///
/// 这些错误反映的是业务约束被打破的情况（例如会话不存在），
/// 而不是基础设施故障（例如数据库连接断开）。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChatError {
    #[error("Session '{0}' not found")]
    SessionNotFound(String),

    #[error("Session '{0}' has no title generated, cannot resume")]
    TitleNotGenerated(String),

    #[error("API key not configured")]
    ApiKeyNotConfigured,

    #[error("Invalid API key")]
    InvalidApiKey,
}
