use anyhow::Result as AnyhowResult;
use chat_pm_database::{DbError, MemoryDb};
use chat_pm_deepseek::{ApiError, ChatRequestConfig, Client as DeepseekClient, ReasoningEffort};
use tokio::sync::mpsc;
use tracing::{debug, info};

use chat_pm_session::{
    ChatError,
    chat::{MessageFrame, ReplyReceiver, StopReason},
    context::Context,
    message::UserInput,
    prompt::{PromptComposer, SystemPrompt, TitlePrompt},
    session::{NewSession, Session, SessionId, Title},
};

// ── PipelineError ───────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("{0}")]
    Domain(#[from] ChatError),
    #[error("{0}")]
    Db(#[from] DbError),
    #[error("{0}")]
    Api(#[from] ApiError),
    #[error("{0}")]
    Internal(#[from] anyhow::Error),
}

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
    pub fn validate(&self) -> AnyhowResult<()> {
        if self.reply_token_limit == 0 {
            return Err(anyhow::anyhow!("reply_token_limit must be > 0"));
        }
        Ok(())
    }

    pub fn set_reasoning_effort_from_str(&mut self, value: &str) -> AnyhowResult<()> {
        self.reasoning_effort = Some(ReasoningEffort::parse(value)?);
        Ok(())
    }

    pub fn load_from_env(mut self) -> AnyhowResult<Self> {
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

// ── Title config (constant for the lightweight title generation request) ─

fn title_request_config(model: &str) -> ChatRequestConfig {
    ChatRequestConfig {
        model: model.to_string(),
        max_tokens: 32,
        thinking_enabled: false,
        reasoning_effort: None,
    }
}

fn clean_title(raw: &str) -> Title {
    Title::new(
        raw.trim()
            .trim_matches(['"', '\'', '《', '》', '「', '」'])
            .to_string(),
    )
}

// ── ChatPipeline ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ChatPipeline {
    client: DeepseekClient,
    db: MemoryDb,
    config: PipelineConfig,
}

impl ChatPipeline {
    pub fn new(db: MemoryDb, client: DeepseekClient, config: PipelineConfig) -> Result<Self, PipelineError> {
        config.validate()?;
        Ok(Self { client, db, config })
    }

    pub fn with_default_deepseek(db: MemoryDb, config: PipelineConfig) -> Result<Self, PipelineError> {
        let client = DeepseekClient::from_env()?;
        Self::new(db, client, config)
    }

    // ── Lifecycle: NewSession ───────────────────────────────────────

    /// 创建新会话 → `NewSession`。
    ///
    /// 数据库记录已写入，但标题尚未生成。
    pub fn create_session(&self) -> Result<NewSession, PipelineError> {
        let session_id = SessionId::new();
        self.db.create_session(&session_id.to_string())?;
        info!(%session_id, "新会话已创建");
        Ok(NewSession { session_id })
    }

    // ── Lifecycle: TitlePrompt → Session ────────────────────────────

    /// `TitlePrompt` → `Session`：调用 LLM 生成标题，持久化后转入正式会话。
    ///
    /// 消耗 `TitlePrompt`，确保标题生成只发生一次。
    pub async fn finalize_session(&self, tp: TitlePrompt) -> Result<Session, PipelineError> {
        let session_id = tp.session_id();
        let messages = tp.compose();

        let raw_title = self
            .client
            .chat_complete(&title_request_config(&self.config.chat_model), &messages)
            .await?;

        let title = clean_title(&raw_title);
        let sid = session_id.to_string();
        self.db.set_session_title(&sid, title.as_str())?;
        info!(session_id = %sid, %title, "会话标题已生成");
        Ok(Session { session_id, title })
    }

    /// 从持久化记录恢复 `Session`（仅限已有标题的会话）。
    pub fn resume_session(&self, session_id: SessionId) -> Result<Session, PipelineError> {
        let sid = session_id.to_string();
        let record = self
            .db
            .get_session(&sid)?
            .ok_or(ChatError::SessionNotFound(sid.clone()))?;
        let title = record
            .title
            .ok_or(ChatError::TitleNotGenerated(sid))?;
        Ok(Session::resume(session_id, Title::new(title)))
    }

    /// 删除会话及其所有聊天记录。
    pub fn delete_session(&self, session_id: SessionId) -> Result<bool, PipelineError> {
        let sid = session_id.to_string();
        let deleted = self.db.delete_session(&sid)?;
        if deleted {
            info!(%sid, "会话已删除");
        }
        Ok(deleted)
    }

    // ── Chat on Session ─────────────────────────────────────────────

    /// 在 `Session` 上进行对话，返回流式消息接收器。
    ///
    /// 类型系统保证：只有已生成标题的 `Session` 才能对话。
    pub async fn chat(
        &self,
        session: &Session,
        user_input: UserInput,
    ) -> Result<mpsc::Receiver<Result<MessageFrame, ApiError>>, PipelineError> {
        let sid = session.session_id.to_string();
        info!(%sid, "开始处理");

        let recent_memory = self
            .db
            .load_recent_memory(&sid, self.config.short_term_turns)?;
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
            if let Err(e) = db.append_chat_turn(&sid, user_input.into(), answer.display_text.clone()) {
                tracing::error!(%sid, error = %e, "保存聊天记录失败");
            }

            if let Ok(stats) = db.stats() {
                info!(
                    sessions = stats.session_count,
                    total_turns = stats.total_turn_count,
                    "记忆写回完成"
                );
            }
        });

        Ok(rx)
    }
}
