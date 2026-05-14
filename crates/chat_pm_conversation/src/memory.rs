use crate::TurnId;

#[derive(Debug, Clone)]
pub struct Memory {
    pub turn: TurnId,
    /// 用户发言
    pub user_text: String,
    /// 助手回复
    pub assistant_text: String,
}

impl Memory {}
