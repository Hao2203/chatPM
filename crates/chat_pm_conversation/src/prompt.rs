use crate::{
    context::Context,
    language::Language,
    message::{ChatMessage, UserInput},
};

/// 系统提示
#[derive(Debug, Clone, Default)]
pub struct SystemPrompt {
    pub role_description: Option<String>,
    pub reply_language: Option<Language>,
}

impl SystemPrompt {
    /// 渲染为单条 system ChatMessage
    pub fn to_message(&self) -> ChatMessage {
        let mut content = String::new();

        if let Some(role_desc) = &self.role_description {
            content.push_str(role_desc);
            content.push('\n');
        }

        if let Some(lang) = &self.reply_language {
            content.push_str(&format!("reply_language：{}", lang.code()));
            content.push('\n');
        }

        ChatMessage::system(content)
    }
}

#[derive(Debug, Clone)]
pub struct PromptComposer {
    system_prompt: SystemPrompt,
}

impl PromptComposer {
    pub fn new(system_prompt: SystemPrompt) -> Self {
        Self { system_prompt }
    }

    pub fn compose_prompt(&self, ctx: Context, user_input: UserInput) -> Vec<ChatMessage> {
        let mut messages = if ctx.recent_memory.is_empty() {
            vec![self.system_prompt.to_message()]
        } else {
            vec![]
        };
        if let Some(summary) = ctx.summary {
            messages.push(ChatMessage::system(format!("Summary: {}", summary.content)));
        }

        for memory in ctx.recent_memory {
            let assistant_msg = ChatMessage::assistant(memory.assistant_text);
            let user_msg = ChatMessage::user(memory.user_text);
            messages.push(assistant_msg);
            messages.push(user_msg);
        }

        messages.push(ChatMessage::user(user_input.into_inner()));

        messages
    }
}
