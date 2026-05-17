use crate::{
    context::Context,
    language::Language,
    message::{ChatMessage, UserInput},
    session::SessionId,
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

// ── Title generation ──────────────────────────────────────────────────

/// 标题生成状态：由 [`NewSession`] 转换而来，承载生成标题所需的全部信息。
///
/// 生命周期：`NewSession` → `TitlePrompt` → `Session`
///
/// [`NewSession`]: crate::session::NewSession
/// [`Session`]: crate::session::Session
#[derive(Debug, Clone)]
pub struct TitlePrompt {
    session_id: SessionId,
    user_input: String,
}

impl TitlePrompt {
    pub fn new(session_id: SessionId, user_input: String) -> Self {
        Self {
            session_id,
            user_input,
        }
    }

    pub fn session_id(&self) -> SessionId {
        self.session_id
    }

    /// 将标题生成需求组装为完整的消息列表，供 LLM 调用方使用。
    pub fn compose(&self) -> Vec<ChatMessage> {
        vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(Self::user_prompt(&self.user_input)),
        ]
    }

    fn system_prompt() -> String {
        "你是一个标题生成助手，只输出标题文本，不输出任何其他内容。".to_string()
    }

    fn user_prompt(user_input: &str) -> String {
        format!(
            "根据以下对话内容，生成一个简洁的标题（不超过10个字，不要加引号）：\n{}",
            user_input
        )
    }
}
