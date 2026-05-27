#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TurnId(uuid::Uuid);

impl TurnId {
    /// 生成新的 TurnId（UUID v7，时间有序，无需数据库）。
    pub fn generate() -> Self {
        Self(uuid::Uuid::now_v7())
    }

    pub fn from_uuid(uuid: uuid::Uuid) -> Self {
        Self(uuid)
    }

    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

impl std::fmt::Display for TurnId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndOfSequence,
    MaxTokens,
    ContentFilter,
}

pub struct MessageFrame {
    pub content: String,
    pub prompt_tokens: Option<usize>,
    pub completion_tokens: Option<usize>,
}

impl MessageFrame {
    pub fn has_token_info(&self) -> bool {
        self.prompt_tokens.is_some()
    }
}

#[derive(Debug, Default)]
pub struct ReplyReceiver {
    content: String,
}

impl ReplyReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn receive(&mut self, text: &str) -> MessageFrame {
        self.content.push_str(text);
        MessageFrame {
            content: text.into(),
            prompt_tokens: None,
            completion_tokens: None,
        }
    }

    pub fn finish(self, stop_reason: StopReason, completion_tokens: usize) -> FinalAnswer {
        let truncation_warning = if stop_reason == StopReason::MaxTokens {
            Some("Response truncated due to length limit. Please try a more specific question.".to_string())
        } else {
            None
        };

        FinalAnswer {
            display_text: self.content.clone(),
            truncation_warning,
            memory_update_plan: MemoryUpdatePlan {
                content_to_store: self.content,
            },
            completion_tokens,
        }
    }
}

#[derive(Debug)]
pub struct MemoryUpdatePlan {
    pub content_to_store: String,
}

#[derive(Debug)]
pub struct FinalAnswer {
    pub completion_tokens: usize,
    pub display_text: String,
    pub truncation_warning: Option<String>,
    pub memory_update_plan: MemoryUpdatePlan,
}
