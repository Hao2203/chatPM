#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnId(pub u64);

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

#[derive(Debug, Default)]
pub struct ReplyReceiver {
    content: String,
}

impl ReplyReceiver {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn receive(&mut self, text: &str) {
        self.content.push_str(text);
    }

    pub fn finish(self, stop_reason: StopReason, completion_tokens: usize) -> LlmResponse {
        LlmResponse {
            raw_text: self.content,
            completion_tokens,
            stop_reason,
        }
    }
}

#[derive(Debug)]
pub struct LlmResponse {
    pub raw_text: String,
    pub completion_tokens: usize,
    pub stop_reason: StopReason,
}

#[derive(Debug)]
pub struct MemoryUpdatePlan {
    pub content_to_store: String,
}

#[derive(Debug)]
pub struct FinalAnswer {
    pub display_text: String,
    pub truncation_warning: Option<String>,
    pub memory_update_plan: MemoryUpdatePlan,
}

pub fn finalize(response: LlmResponse) -> FinalAnswer {
    let truncation_warning = if response.stop_reason == StopReason::MaxTokens {
        Some("回答因长度限制被截断，请尝试更具体的问题。".to_string())
    } else {
        None
    };

    FinalAnswer {
        display_text: response.raw_text.clone(),
        truncation_warning,
        memory_update_plan: MemoryUpdatePlan {
            content_to_store: response.raw_text,
        },
    }
}
