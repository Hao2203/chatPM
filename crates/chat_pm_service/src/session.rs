use anyhow::Result as AnyhowResult;
use std::sync::Arc;

use chat_pm_database::{ChatDb, DbError};
use chat_pm_deepseek::{
    ApiError, ChatRequestConfig, Client as DeepseekClient, DeepSeekModel, ReasoningEffort,
};
use chat_pm_knowledge::KnowledgeError;
use chat_pm_sync::DeviceId;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::knowledge::KnowledgeService;

use chat_pm_session::{
    ChatError,
    chat::MessageFrame,
    memory::Memory,
    message::UserInput,
    prompt::{Context, PromptComposer, SummaryPrompt, SystemPrompt, TitlePrompt},
    session::{NewSession, Session, SessionId, Title},
    summarization,
    summarization::Summary,
};

// ── CommandError ───────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum CommandError {
    #[error("[Chat Error] {0}")]
    Chat(#[from] ChatError),
    #[error("[Database Error] {0}")]
    Db(#[from] DbError),
    #[error("[API Error] {0}")]
    Api(#[from] ApiError),
    #[error("[Knowledge Error] {0}")]
    Knowledge(#[from] KnowledgeError),
    #[error("[Internal Error] {0}")]
    Internal(#[from] anyhow::Error),
}

#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub chat_model: DeepSeekModel,
    pub token_limit: usize,
    pub reply_token_limit: usize,
    pub short_term_turns: usize,
    pub long_term_top_k: usize,
    pub context_window: usize,
    pub summary_ratio: f64,
    pub system_role: String,
    pub thinking_enabled: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
    pub device_id: DeviceId,
    /// 从知识库检索的最大块数。
    pub knowledge_top_k: usize,
}

impl ChatConfig {
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

    pub fn set_chat_model(&mut self, model: &str) -> AnyhowResult<()> {
        self.chat_model = model.parse()?;
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

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            chat_model: DeepSeekModel::V4Flash,
            token_limit: 8192,
            reply_token_limit: 2048,
            short_term_turns: 6,
            long_term_top_k: 4,
            context_window: 1_048_576, // DeepSeek V4 Flash 1M
            summary_ratio: 0.6,
            system_role: "你是一名智能助手，能够记住对话历史并提供连贯的回答。".to_string(),
            thinking_enabled: false,
            reasoning_effort: None,
            device_id: DeviceId::generate(),
            knowledge_top_k: 5,
        }
    }
}

// ── Title config (constant for the lightweight title generation request) ─

fn title_request_config(model: DeepSeekModel) -> ChatRequestConfig {
    ChatRequestConfig {
        model,
        max_tokens: 32,
        thinking_enabled: false,
        reasoning_effort: None,
    }
}

// ── Summary config (constant for the lightweight summary generation request) ─

fn summary_request_config(model: DeepSeekModel) -> ChatRequestConfig {
    ChatRequestConfig {
        model,
        max_tokens: 512,
        thinking_enabled: false,
        reasoning_effort: None,
    }
}

// ── ChatService ────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ChatService {
    client: DeepseekClient,
    db: ChatDb,
    config: ChatConfig,
    knowledge_service: Option<Arc<KnowledgeService>>,
}

impl ChatService {
    pub fn new(
        db: ChatDb,
        client: DeepseekClient,
        config: ChatConfig,
    ) -> Result<Self, CommandError> {
        config.validate()?;
        Ok(Self { client, db, config, knowledge_service: None })
    }

    pub fn with_default_deepseek(db: ChatDb, config: ChatConfig) -> Result<Self, CommandError> {
        let client = DeepseekClient::from_env()?;
        Self::new(db, client, config)
    }

    /// 设置知识库服务。
    pub fn set_knowledge_service(&mut self, ks: Arc<KnowledgeService>) {
        self.knowledge_service = Some(ks);
    }

    // ── Lifecycle: NewSession ───────────────────────────────────────

    /// 创建新会话 → `NewSession`。
    ///
    /// 数据库记录已写入，但标题尚未生成。
    pub fn create_session(&self) -> Result<NewSession, CommandError> {
        let session_id = SessionId::new();
        self.db.create_session(session_id)?;
        info!(%session_id, "New session created");
        Ok(NewSession::with_id(session_id))
    }

    // ── Lifecycle: TitlePrompt → Session ────────────────────────────

    /// `TitlePrompt` → `Session`：调用 LLM 生成标题，持久化后转入正式会话。
    ///
    /// 消耗 `TitlePrompt`，确保标题生成只发生一次。
    pub async fn finalize_session(&self, tp: TitlePrompt<'_>) -> Result<Session, CommandError> {
        let session_id = tp.session_id();
        let messages = tp.compose();

        let raw_title = self
            .client
            .chat_complete(&title_request_config(self.config.chat_model), &messages)
            .await?;

        let title = Title::new(raw_title);
        self.db.set_session_title(session_id, title.as_str())?;
        info!(session_id = %session_id, %title, "Session title generated");
        Ok(Session::resume(session_id, title))
    }

    /// 从持久化记录恢复 `Session`（仅限已有标题的会话）。
    pub fn resume_session(&self, session_id: SessionId) -> Result<Session, CommandError> {
        let record = self
            .db
            .get_session(session_id)?
            .ok_or(ChatError::SessionNotFound(session_id.to_string()))?;
        let title = record
            .title
            .ok_or(ChatError::TitleNotGenerated(session_id.to_string()))?;
        Ok(Session::resume(session_id, Title::new(title)))
    }

    /// 删除会话及其所有聊天记录。
    pub fn delete_session(&self, session_id: SessionId) -> Result<bool, CommandError> {
        let deleted = self.db.delete_session(session_id)?;
        if deleted {
            info!(%session_id, "Session deleted");
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
    ) -> Result<mpsc::Receiver<Result<MessageFrame, ApiError>>, CommandError> {
        let sid = session.session_id();
        info!(%sid, "Processing started");

        // 载入摘要（如有）
        let summary = self
            .db
            .get_summary(sid)?
            .map(|(content, last_turn_uuid, last_turn_num)| Summary {
                content,
                last_turn_id: chat_pm_session::TurnId::from_uuid(last_turn_uuid),
                last_turn_num: last_turn_num as u64,
            });

        let recent_memory = self
            .db
            .load_recent_memory(sid, self.config.short_term_turns)?;
        debug!(count = recent_memory.len(), "Short-term memory loaded");

        let system_prompt = SystemPrompt {
            role_description: Some(self.config.system_role.clone()),
            ..SystemPrompt::default()
        };

        // 检索知识库上下文
        let knowledge = if let Some(ref ks) = self.knowledge_service {
            match ks
                .retrieve_context(session.session_id(), user_input.as_str(), self.config.knowledge_top_k)
                .await
            {
                Ok(results) => {
                    debug!(results = results.len(), "知识库检索完成");
                    group_search_results_by_kb(results)
                }
                Err(e) => {
                    warn!(error = %e, "知识库检索失败，继续对话");
                    vec![]
                }
            }
        } else {
            vec![]
        };

        let ctx = Context {
            summary,
            recent_memory,
            knowledge,
        };
        let composer = PromptComposer::new(system_prompt);
        let messages = composer.compose_prompt(ctx, &user_input);
        debug!(msgs = messages.len(), "Prompt assembled");

        let mut stream = self
            .client
            .stream_chat(
                &ChatRequestConfig {
                    model: self.config.chat_model,
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
                sid,
                user_input.into_inner(),
                assistant_text,
                Some(prompt_tokens_result as i64),
                Some(completion_tokens_result as i64),
                config.device_id,
            ) {
                tracing::error!(%sid, error = %e, "Failed to save chat record");
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
            ) && let Err(e) = summarize_session_inner(&db, &client, &config, sid).await
            {
                tracing::error!(%sid, error = %e, "Summary generation failed");
            }
        });

        Ok(rx)
    }

    /// 手动触发指定会话的摘要压缩（对外暴露，也可用于测试）。
    pub async fn summarize_session(&self, session_id: SessionId) -> Result<(), CommandError> {
        summarize_session_inner(&self.db, &self.client, &self.config, session_id).await
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

/// 将搜索结果按文档标题分组为 `KnowledgeContext` 列表。
fn group_search_results_by_kb(
    results: Vec<chat_pm_knowledge::SearchResult>,
) -> Vec<chat_pm_session::prompt::KnowledgeContext> {
    use std::collections::HashMap;
    let mut groups: HashMap<String, Vec<chat_pm_session::prompt::KnowledgeChunk>> = HashMap::new();

    for r in results {
        let entry = groups.entry(r.document_id.clone()).or_default();
        entry.push(chat_pm_session::prompt::KnowledgeChunk {
            content: r.content,
            document_title: r.document_id.clone(),
            score: r.score,
        });
    }

    groups
        .into_iter()
        .map(|(title, chunks)| chat_pm_session::prompt::KnowledgeContext {
            kb_name: title,
            chunks,
        })
        .collect()
}

// ── Standalone summarization function (usable inside tokio::spawn) ─────

async fn summarize_session_inner(
    db: &ChatDb,
    client: &DeepseekClient,
    config: &ChatConfig,
    sid: SessionId,
) -> Result<(), CommandError> {
    let total_turns = db.count_turns(sid)?;
    let existing = db.get_summary(sid)?;
    let existing_summary = existing.as_ref().map(|(c, last_uuid, last_num)| Summary {
        content: c.clone(),
        last_turn_id: chat_pm_session::TurnId::from_uuid(*last_uuid),
        last_turn_num: *last_num as u64,
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

    let prompt = SummaryPrompt::new(existing.as_ref().map(|(c, _, _)| c.clone()), new_turns);
    let messages = prompt.compose();

    let new_content = client
        .chat_complete(&summary_request_config(config.chat_model), &messages)
        .await?;

    let trimmed_content = new_content.trim().to_string();
    if trimmed_content.is_empty() {
        tracing::warn!(%sid, "Summary generation produced empty result, skipping update");
        return Ok(());
    }

    let turn_uuid = db.turn_id_by_num(sid, plan.new_last_turn_num)?.as_uuid();
    db.upsert_summary(
        sid,
        &trimmed_content,
        plan.new_last_turn_num as i64,
        turn_uuid,
    )?;
    info!(%sid, last_turn_num = plan.new_last_turn_num, "Summary updated");
    Ok(())
}
