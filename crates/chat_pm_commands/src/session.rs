use anyhow::Result as AnyhowResult;
use chat_pm_database::{DbError, MemoryDb};
use chat_pm_deepseek::{ApiError, ChatRequestConfig, Client as DeepseekClient, ReasoningEffort};
use tokio::sync::mpsc;
use tracing::{debug, info};

use chat_pm_session::{
    ChatError,
    chat::MessageFrame,
    context::Context,
    memory::Memory,
    message::UserInput,
    prompt::{PromptComposer, SummaryPrompt, SystemPrompt, TitlePrompt},
    session::{NewSession, Session, SessionId, Title},
    summarization,
    summary::Summary,
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
    pub context_window: usize,
    pub summary_ratio: f64,
    pub system_role: String,
    pub thinking_enabled: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
}

impl PipelineConfig {
    pub fn validate(&self) -> AnyhowResult<()> {
        if self.reply_token_limit == 0 {
            return Err(anyhow::anyhow!("reply_token_limit must be > 0"));
        }
        if self.context_window == 0 {
            return Err(anyhow::anyhow!("context_window must be > 0"));
        }
        if !(0.0..=1.0).contains(&self.summary_ratio) {
            return Err(anyhow::anyhow!("summary_ratio must be between 0.0 and 1.0"));
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
            context_window: 1_048_576,     // DeepSeek V4 Flash 1M
            summary_ratio: 0.6,
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

// ── Summary config (constant for the lightweight summary generation request) ─

fn summary_request_config(model: &str) -> ChatRequestConfig {
    ChatRequestConfig {
        model: model.to_string(),
        max_tokens: 512,
        thinking_enabled: false,
        reasoning_effort: None,
    }
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

        // 载入摘要（如有）
        let summary = self.db.get_summary(&sid)?.map(|(content, last_turn_num)| {
            Summary {
                content,
                last_turn_id: chat_pm_session::TurnId(last_turn_num as u64),
            }
        });

        let recent_memory = self
            .db
            .load_recent_memory(&sid, self.config.short_term_turns)?;
        debug!(count = recent_memory.len(), "短期记忆加载完成");

        let system_prompt = SystemPrompt {
            role_description: Some(self.config.system_role.clone()),
            ..SystemPrompt::default()
        };

        let ctx = Context {
            summary,
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
        let config = self.config.clone();
        let client = self.client.clone();
        tokio::spawn(async move {
            let mut prompt_tokens_result = 0;
            let mut completion_tokens_result = 0;
            let mut assistant_text = String::new();

            while let Some(result) = stream.recv().await {
                let chunk = match result {
                    Ok(c) => c,
                    Err(e) => {
                        let _ = tx.send(Err(e)).await;
                        return;
                    }
                };

                if chunk.prompt_tokens > 0 {
                    prompt_tokens_result = chunk.prompt_tokens;
                }
                if chunk.completion_tokens > 0 {
                    completion_tokens_result = chunk.completion_tokens;
                }

                assistant_text.push_str(&chunk.raw_text);

                if tx
                    .send(Ok(MessageFrame {
                        content: chunk.raw_text,
                        prompt_tokens: None,
                        completion_tokens: None,
                    }))
                    .await
                    .is_err()
                {
                    return;
                }
            }

            // 保存轮次到 DB
            if let Err(e) = db.append_chat_turn(
                &sid,
                user_input.into(),
                assistant_text,
                Some(prompt_tokens_result as i64),
                Some(completion_tokens_result as i64),
            ) {
                tracing::error!(%sid, error = %e, "保存聊天记录失败");
            }

            // 发送最后一帧携带 token 信息（在保存之后，确保前端收到时数据已持久化）
            if tx
                .send(Ok(MessageFrame {
                    content: String::new(),
                    prompt_tokens: Some(prompt_tokens_result),
                    completion_tokens: Some(completion_tokens_result),
                }))
                .await
                .is_err()
            {
                return;
            }

            // 检查是否需要触发摘要
            if summarization::should_summarize(
                prompt_tokens_result,
                config.context_window,
                config.summary_ratio,
            ) {
                if let Err(e) = summarize_session_inner(&db, &client, &config, &sid).await {
                    tracing::error!(%sid, error = %e, "摘要生成失败");
                }
            }
        });

        Ok(rx)
    }

    /// 手动触发指定会话的摘要压缩（对外暴露，也可用于测试）。
    pub async fn summarize_session(&self, session_id: SessionId) -> Result<(), PipelineError> {
        let sid = session_id.to_string();
        summarize_session_inner(&self.db, &self.client, &self.config, &sid).await
    }
}

// ── Standalone summarization function (usable inside tokio::spawn) ─────

async fn summarize_session_inner(
    db: &MemoryDb,
    client: &DeepseekClient,
    config: &PipelineConfig,
    sid: &str,
) -> Result<(), PipelineError> {
    let total_turns = db.count_turns(sid)?;
    let existing = db.get_summary(sid)?;
    let existing_summary = existing.as_ref().map(|(c, last_num)| Summary {
        content: c.clone(),
        last_turn_id: chat_pm_session::TurnId(*last_num as u64),
    });

    let plan = match summarization::plan_summarization(
        total_turns,
        config.short_term_turns,
        existing_summary.as_ref(),
    ) {
        Some(p) => p,
        None => return Ok(()),
    };

    let (from, to) = summarization::turn_range_to_summarize(existing_summary.as_ref(), &plan);
    let new_turns: Vec<Memory> = db.get_turns_range(sid, from, to)?;

    if new_turns.is_empty() && existing.is_none() {
        return Ok(());
    }

    let prompt = SummaryPrompt::new(
        existing.map(|(c, _)| c),
        new_turns,
    );
    let messages = prompt.compose();

    let new_content = client
        .chat_complete(&summary_request_config(&config.chat_model), &messages)
        .await?;

    let trimmed_content = new_content.trim().to_string();
    if trimmed_content.is_empty() {
        tracing::warn!(%sid, "摘要生成为空，跳过更新");
        return Ok(());
    }

    db.upsert_summary(sid, &trimmed_content, plan.new_last_turn_num as i64)?;
    info!(%sid, last_turn_num = plan.new_last_turn_num, "摘要已更新");
    Ok(())
}
