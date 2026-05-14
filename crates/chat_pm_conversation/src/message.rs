use derive_more::Into;

use crate::Role;

#[derive(Debug, Clone, Into)]
pub struct UserInput(String);

impl UserInput {
    pub fn new(content: &str) -> Self {
        let content = content.split_whitespace().collect::<Vec<_>>().join(" ");
        Self(content)
    }

    pub fn into_inner(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}
