use crate::{
    language::Language,
    memory::Memory,
    message::{ChatMessage, UserInput},
    session::SessionId,
    summarization::Summary,
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

/// 从知识库检索到的上下文片段。
#[derive(Debug, Clone)]
pub struct KnowledgeChunk {
    pub content: String,
    pub document_title: String,
    pub score: f32,
}

/// 单个知识库的检索结果。
#[derive(Debug, Clone)]
pub struct KnowledgeContext {
    pub kb_name: String,
    pub chunks: Vec<KnowledgeChunk>,
}

#[derive(Debug, Clone)]
pub struct Context {
    pub summary: Option<Summary>,
    pub recent_memory: Vec<Memory>,
    /// 从知识库检索到的上下文。
    pub knowledge: Vec<KnowledgeContext>,
}

#[derive(Debug, Clone)]
pub struct PromptComposer {
    system_prompt: SystemPrompt,
}

impl PromptComposer {
    pub fn new(system_prompt: SystemPrompt) -> Self {
        Self { system_prompt }
    }

    pub fn compose_prompt(&self, ctx: Context, user_input: &UserInput) -> Vec<ChatMessage> {
        let mut messages = if ctx.recent_memory.is_empty() {
            vec![self.system_prompt.to_message()]
        } else {
            vec![]
        };

        // 注入知识库上下文（在系统提示词之后、摘要/记忆之前）
        if !ctx.knowledge.is_empty() {
            let mut kb_text = String::from(
                "以下是从知识库中检索到的相关信息，请参考这些内容来回答问题：\n\n",
            );
            for kc in &ctx.knowledge {
                kb_text.push_str(&format!("## 知识库「{}」中的相关内容：\n", kc.kb_name));
                for chunk in &kc.chunks {
                    kb_text.push_str(&format!(
                        "【来源：{}，相关度：{:.2}】\n{}\n\n",
                        chunk.document_title, chunk.score, chunk.content
                    ));
                }
            }
            messages.push(ChatMessage::system(kb_text));
        }

        if let Some(summary) = ctx.summary {
            messages.push(ChatMessage::system(format!("Summary: {}", summary.content)));
        }

        for memory in ctx.recent_memory {
            let assistant_msg = ChatMessage::assistant(memory.assistant_text);
            let user_msg = ChatMessage::user(memory.user_text);
            messages.push(assistant_msg);
            messages.push(user_msg);
        }

        messages.push(ChatMessage::user(user_input.as_str()));

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
pub struct TitlePrompt<'a> {
    session_id: SessionId,
    user_input: &'a str,
}

impl<'a> TitlePrompt<'a> {
    pub fn new(session_id: SessionId, user_input: &'a str) -> Self {
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
            ChatMessage::user(Self::user_prompt(self.user_input)),
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

// ── Summary generation ──────────────────────────────────────────────────

/// 摘要生成请求。包含现有摘要（如有）和需要纳入的新轮次。
///
/// 调用方先通过 `turn_range_to_summarize()` 计算出轮次范围，
/// 从 DB 加载对应轮次后构造此类型，再调用 `compose()` 获取 LLM 消息列表。
#[derive(Debug, Clone)]
pub struct SummaryPrompt {
    existing_summary: Option<String>,
    turns: Vec<Memory>,
}

impl SummaryPrompt {
    pub fn new(existing_summary: Option<String>, turns: Vec<Memory>) -> Self {
        Self {
            existing_summary,
            turns,
        }
    }

    /// 组装为完整的消息列表，供 LLM 调用方使用。
    pub fn compose(&self) -> Vec<ChatMessage> {
        let mut content = String::new();

        if let Some(ref summary) = self.existing_summary {
            content.push_str("## 现有摘要\n");
            content.push_str(summary);
            content.push_str("\n\n");
        }

        content.push_str("## 新的对话内容\n");
        for (i, turn) in self.turns.iter().enumerate() {
            content.push_str(&format!("用户: {}\n", turn.user_text));
            content.push_str(&format!("助手: {}\n", turn.assistant_text));
            if i < self.turns.len() - 1 {
                content.push('\n');
            }
        }

        vec![
            ChatMessage::system(Self::system_prompt()),
            ChatMessage::user(content),
        ]
    }

    fn system_prompt() -> String {
        "你是一个专业的对话摘要助手。你的任务是将对话内容浓缩为简洁但信息完整的摘要。\
         保留所有关键信息、用户偏好、重要决定和具体细节。\
         如果提供了现有摘要，请将其与新内容融合，生成一份连贯的更新后摘要。\
         如果新对话与现有摘要内容重复，保留信息更完整的一方。\
         只输出摘要文本，不输出任何其他内容。"
            .to_string()
    }
}
