use anyhow::{Result, anyhow};
use chat_pm_database::MemoryDb;
use chat_pm_deepseek::{ChatRequestConfig, Client as DeepseekClient, ReasoningEffort};
use tokio::sync::mpsc;
use tracing::{debug, info};
use uuid::Uuid;

use chat_pm_session::{
    chat::{MessageFrame, ReplyReceiver, StopReason},
    context::Context,
    message::{ChatMessage, UserInput},
    prompt::{PromptComposer, SystemPrompt},
};

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub chat_model: String,
    pub token_limit: usize,
    pub reply_token_limit: usize,
    pub short_term_turns: usize,
    pub long_term_top_k: usize,
    pub system_role: String,
    pub thinking_enabled: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl PipelineConfig {
    pub fn validate(&self) -> Result<()> {
        if self.reply_token_limit == 0 {
            return Err(anyhow!("reply_token_limit must be > 0"));
        }
        Ok(())
    }

    pub fn set_reasoning_effort_from_str(&mut self, value: &str) -> Result<()> {
        self.reasoning_effort = Some(ReasoningEffort::parse(value)?);
        Ok(())
    }

    pub fn load_from_env(mut self) -> Result<Self> {
        if let Ok(value) = std::env::var("CHAT_PM_REASONING_EFFORT") {
            self.set_reasoning_effort_from_str(&value)?;
        }
        self.validate()?;
        Ok(self)
    }
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            chat_model: "deepseek-v4-flash".to_string(),
            token_limit: 8192,
            reply_token_limit: 2048,
            short_term_turns: 6,
            long_term_top_k: 4,
            system_role: "你是一名智能助手，能够记住对话历史并提供连贯的回答。".to_string(),
            thinking_enabled: false,
            reasoning_effort: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct SessionId(Uuid);

impl SessionId {
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

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

#[derive(Clone)]
pub struct ChatPipeline {
    client: DeepseekClient,
    db: MemoryDb,
    config: PipelineConfig,
}

impl ChatPipeline {
    pub fn new(db: MemoryDb, client: DeepseekClient, config: PipelineConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self { client, db, config })
    }

    pub fn with_default_deepseek(db: MemoryDb, config: PipelineConfig) -> Result<Self> {
        let client = DeepseekClient::from_env()?;
        Self::new(db, client, config)
    }

    pub fn create_session(&self) -> SessionHandle {
        let session_id = Uuid::now_v7();
        self.db.create_session(&session_id.to_string());
        info!(%session_id, "新会话已创建");
        SessionHandle(session_id)
    }

    pub fn resume_session(&self, session_id: SessionId) -> Result<SessionHandle> {
        if self.db.session_exists(&session_id.to_string()) {
            Ok(SessionHandle(session_id.0))
        } else {
            Err(anyhow!("session '{}' 不存在", session_id))
        }
    }

    pub fn set_session_title(&self, handle: &SessionHandle, title: &str) {
        let session_id = handle.id().to_string();
        self.db.set_session_title(&session_id, title);
        info!(%session_id, %title, "会话标题已更新");
    }

    /// 根据用户首条消息生成会话标题（仅在新会话的首轮调用）
    async fn generate_title(&self, session_id: &str, user_input: &UserInput) -> Result<()> {
        let prompt = format!(
            "根据以下对话内容，生成一个简洁的标题（不超过10个字，不要加引号）：\n{}",
            user_input
        );
        let messages = vec![
            ChatMessage::system("你是一个标题生成助手，只输出标题文本，不输出任何其他内容。"),
            ChatMessage::user(prompt),
        ];
        let title = self
            .client
            .chat_complete(
                &ChatRequestConfig {
                    model: self.config.chat_model.clone(),
                    max_tokens: 32,
                    thinking_enabled: false,
                    reasoning_effort: None,
                },
                &messages,
            )
            .await?;
        let title = title.trim().trim_matches(&['"', '\'', '《', '》', '「', '」']).to_string();
        self.db.set_session_title(session_id, &title);
        info!(%session_id, %title, "标题已生成");
        Ok(())
    }

    pub async fn chat(
        &self,
        handle: &SessionHandle,
        user_input: UserInput,
    ) -> Result<mpsc::Receiver<Result<MessageFrame>>> {
        let session_id = handle.id().to_string();
        info!(%session_id, "开始处理");

        // 首轮对话：先生成标题，再进行回复
        let recent_memory = self
            .db
            .load_recent_memory(&session_id, self.config.short_term_turns);
        let is_first_turn = recent_memory.is_empty();
        if is_first_turn {
            if let Err(e) = self.generate_title(&session_id, &user_input).await {
                tracing::warn!(%session_id, error = %e, "标题生成失败，继续回复");
            }
        }
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
        let messages = composer.compose_prompt(ctx, user_input.clone());
        debug!(msgs = messages.len(), "Prompt 组装完成");

        let mut stream = self
            .client
            .stream_chat(
                &ChatRequestConfig {
                    model: self.config.chat_model.clone(),
                    max_tokens: self.config.reply_token_limit,
                    thinking_enabled: self.config.thinking_enabled,
                    reasoning_effort: self.config.reasoning_effort,
                },
                &messages,
            )
            .await?;

        let (tx, rx) = mpsc::channel(10);
        let db = self.db.clone();
        tokio::spawn(async move {
            let mut receiver = ReplyReceiver::new();
            let mut stop_reason_result = StopReason::MaxTokens;
            let mut completion_tokens_result = 0;

            while let Some(result) = stream.recv().await {
                let frame = || {
                    let chunk = result?;
                    if let Some(stop_reason) = chunk.stop_reason {
                        stop_reason_result = stop_reason;
                    }
                    completion_tokens_result = chunk.completion_tokens;
                    Ok(receiver.receive(&chunk.raw_text))
                };

                if tx.send(frame()).await.is_err() {
                    return;
                }
            }

            let answer = receiver.finish(stop_reason_result, completion_tokens_result);
            db.append_chat_turn(&session_id, user_input.into(), answer.display_text.clone());

            let stats = db.stats();
            info!(
                sessions = stats.session_count,
                total_turns = stats.total_turn_count,
                "记忆写回完成"
            );
        });

        Ok(rx)
    }
}
