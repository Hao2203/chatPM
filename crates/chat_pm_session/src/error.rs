/// 聊天业务错误：表示违反了聊天应用的核心业务规则。
///
/// 这些错误反映的是业务约束被打破的情况（例如会话不存在），
/// 而不是基础设施故障（例如数据库连接断开）。
#[derive(Debug, Clone, thiserror::Error)]
pub enum ChatError {
    #[error("会话 '{0}' 不存在")]
    SessionNotFound(String),

    #[error("会话 '{0}' 尚未生成标题，无法恢复")]
    TitleNotGenerated(String),

    #[error("未配置 API Key")]
    ApiKeyNotConfigured,

    #[error("无效的 API Key")]
    InvalidApiKey,
}
