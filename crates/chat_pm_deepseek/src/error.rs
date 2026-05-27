/// 外部接口错误：调用 DeepSeek API 时产生的错误。
///
/// 与 `ChatError` 的区别：这些是基础设施/外部依赖的故障，
/// 不是业务逻辑约束的违反。
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("API request failed: {0}")]
    RequestFailed(String),

    #[error("API returned error status: {0}")]
    ErrorStatus(String),

    #[error("API response parse failed: {0}")]
    ParseFailed(String),

    #[error("Model returned no choices")]
    NoChoice,
}
