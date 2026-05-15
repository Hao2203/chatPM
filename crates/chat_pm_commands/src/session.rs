use anyhow::{Context as _, Result, anyhow};
use async_openai::{
    Client,
    types::chat::{
        ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
        ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
        CreateChatCompletionRequestArgs, FinishReason,
    },
};
use chat_pm_database::{MemoryDb, SessionRecord, TurnRecord};
use chrono::Utc;
use tracing::{debug, info};
use uuid::Uuid;

use chat_pm_conversation::{
    chat::{self, FinalAnswer, LlmResponse, Role, StopReason},
    context::Context,
    memory::Memory,
    message::{ChatMessage, UserInput},
    prompt::{PromptComposer, SystemPrompt},
};

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub chat_model: String,
    pub embedding_model: String,
    pub token_limit: usize,
    pub reply_token_limit: usize,
    pub short_term_turns: usize,
    pub long_term_top_k: usize,
    pub system_role: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            chat_model: "deepseek-v4-flash".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            token_limit: 8192,
            reply_token_limit: 2048,
            short_term_turns: 6,
            long_term_top_k: 4,
            system_role: "你是一名智能助手，能够记住对话历史并提供连贯的回答。".to_string(),
        }
    }
}

// ─────────────────────────────────────────
// SessionHandle
// ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(Uuid);

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug)]
pub struct SessionHandle(Uuid);

impl SessionHandle {
    pub fn id(&self) -> SessionId {
        SessionId(self.0)
    }
}

impl std::fmt::Display for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ─────────────────────────────────────────
// 流程编排器
// ─────────────────────────────────────────

#[derive(Clone)]
pub struct ChatPipeline {
    client: Client<chat_pm_deepseek::Config>,
    db: MemoryDb,
    config: PipelineConfig,
}

impl ChatPipeline {
    pub fn new(db: MemoryDb, config: PipelineConfig) -> Self {
        let api_key = std::env::var("DEEPSEEK_API_KEY").expect("DEEPSEEK_API_KEY");
        let deepseek_config = chat_pm_deepseek::Config {
            api_key: chat_pm_deepseek::ApiKey::new(api_key).unwrap(),
        };
        Self {
            client: Client::with_config(deepseek_config),
            db,
            config,
        }
    }

    pub fn create_session(&self) -> SessionHandle {
        let session_id = Uuid::now_v7();
        self.db.upsert_session(SessionRecord {
            session_id: session_id.to_string(),
            created_at: Utc::now(),
            user_persona: None, // user_persona 已移除
        });
        info!(%session_id, "新会话已创建");
        SessionHandle(session_id)
    }

    pub fn resume_session(&self, session_id: SessionId) -> Result<SessionHandle> {
        self.db
            .get_session(&session_id.to_string())
            .map(|_| SessionHandle(session_id.0))
            .ok_or_else(|| anyhow!("session '{}' 不存在", session_id))
    }

    pub async fn chat(&self, handle: &SessionHandle, user_input: UserInput) -> Result<FinalAnswer> {
        let session_id = handle.id().to_string();
        let turn_id = self.db.next_turn_id(&session_id);

        info!(turn = turn_id.0, "开始处理");

        let recent_records = self
            .db
            .recent_turns(&session_id, self.config.short_term_turns);
        let recent_memory: Vec<Memory> = recent_records
            .iter()
            .map(TurnRecord::to_memory_chunk)
            .collect();
        debug!(count = recent_memory.len(), "短期记忆加载完成");

        let system_prompt = SystemPrompt {
            role_description: Some(self.config.system_role.clone()),
            ..SystemPrompt::default()
        };

        let ctx = Context {
            summary: None,
            recent_memory,
        };
        let composer = PromptComposer::new(system_prompt);
        let step4 = composer.compose_prompt(turn_id, ctx, user_input.clone());

        debug!(msgs = step4.messages.len(), "Prompt 组装完成");

        let (raw_text, completion_tokens, stop_reason) = self
            .call_chat_api(&step4.messages)
            .await
            .context("Chat 补全失败")?;
        info!(tokens = completion_tokens, "模型回复完成");

        let llm_response = LlmResponse {
            turn_id,
            raw_text,
            completion_tokens,
            stop_reason,
        };
        let answer = chat::finalize(llm_response);

        self.db.append_turn(TurnRecord {
            turn_id: answer.turn_id,
            session_id: session_id.to_string(),
            user_text: user_input.into(),
            assistant_text: answer.display_text.clone(),
            created_at: Utc::now(),
        });

        let stats = self.db.stats();
        info!(
            sessions = stats.session_count,
            total_turns = stats.total_turn_count,
            "记忆写回完成"
        );

        Ok(answer)
    }

    async fn call_chat_api(&self, messages: &[ChatMessage]) -> Result<(String, usize, StopReason)> {
        // 把 chat::ChatMessage 转换为 async_openai 的类型
        let api_messages: Vec<ChatCompletionRequestMessage> = messages
            .iter()
            .map(|m| -> Result<ChatCompletionRequestMessage> {
                Ok(match m.role {
                    Role::System => ChatCompletionRequestSystemMessageArgs::default()
                        .content(m.content.clone())
                        .build()?
                        .into(),
                    Role::User => ChatCompletionRequestUserMessageArgs::default()
                        .content(m.content.clone())
                        .build()?
                        .into(),
                    Role::Assistant => ChatCompletionRequestAssistantMessageArgs::default()
                        .content(m.content.clone())
                        .build()?
                        .into(),
                })
            })
            .collect::<Result<_>>()?;

        let request = CreateChatCompletionRequestArgs::default()
            .model(self.config.chat_model.clone())
            .messages(api_messages)
            .max_tokens(self.config.reply_token_limit as u32)
            .build()?;

        let response = self.client.chat().create(request).await?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .context("模型未返回 choice")?;

        let raw_text = choice.message.content.unwrap_or_default();
        let completion_tokens = response
            .usage
            .map(|u| u.completion_tokens as usize)
            .unwrap_or(0);
        let stop_reason = match choice.finish_reason {
            Some(FinishReason::Stop) => StopReason::EndOfSequence,
            Some(FinishReason::Length) => StopReason::MaxTokens,
            Some(FinishReason::ContentFilter) => StopReason::ContentFilter,
            _ => StopReason::EndOfSequence,
        };

        Ok((raw_text, completion_tokens, stop_reason))
    }
}
