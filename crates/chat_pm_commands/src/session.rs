/// # 异步流程编排层
use anyhow::{Context, Result, anyhow};
use async_openai::{
    Client,
    types::{
        chat::{
            ChatCompletionRequestAssistantMessageArgs, ChatCompletionRequestMessage,
            ChatCompletionRequestSystemMessageArgs, ChatCompletionRequestUserMessageArgs,
            CreateChatCompletionRequestArgs, FinishReason,
        },
        embeddings::CreateEmbeddingRequest,
    },
};
use chat_pm_database::{MemoryDb, SessionRecord, TurnRecord};
use chrono::Utc;
use tracing::{debug, info, instrument};
use uuid::Uuid;

use chat_pm_conversation::chat::{
    self, ChatMessage, FinalAnswer, MemoryChunk, RawInput, Role, Similarity, StopReason,
    SystemPrompt, TokenCount, TruncationStrategy, TurnId, Vector,
};

// ─────────────────────────────────────────
// 配置
// ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct PipelineConfig {
    pub chat_model: String,
    pub embedding_model: String,
    pub token_limit: TokenCount,
    pub short_term_turns: usize,
    pub long_term_top_k: usize,
    pub truncation_strategy: TruncationStrategy,
    pub system_role: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            chat_model: "deepseek-v4-flash".to_string(),
            embedding_model: "text-embedding-3-small".to_string(),
            token_limit: TokenCount(8192),
            short_term_turns: 6,
            long_term_top_k: 4,
            truncation_strategy: TruncationStrategy::ByRelevance,
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

    #[instrument(skip(self, handle, user_text), fields(session_id = %handle, user_text = %user_text))]
    pub async fn chat(&self, handle: &SessionHandle, user_text: &str) -> Result<FinalAnswer> {
        let session_id = handle.id();
        let turn_id = self.db.next_turn_id(&session_id.to_string());

        info!(turn = turn_id.0, "开始处理");

        // ── Step 1: Embedding ──────────────────────────────────────────
        // let query_vector = self.embed_text(user_text).await.context("Embedding 失败")?;
        // debug!(dim = query_vector.0.len(), "Embedding 完成");

        // ── Step 2: 短期记忆 ───────────────────────────────────────────
        let recent_records = self
            .db
            .recent_turns(&session_id.to_string(), self.config.short_term_turns);
        let short_term_ids: Vec<TurnId> = recent_records.iter().map(|r| r.turn_id).collect();
        let short_term: Vec<MemoryChunk> = recent_records
            .iter()
            .map(|r| TurnRecord::to_memory_chunk(r, Similarity(1.0)))
            .collect();
        debug!(count = short_term.len(), "短期记忆加载完成");

        // ── Step 3: 长期记忆 ───────────────────────────────────────────
        let long_term: Vec<MemoryChunk> = self
            .db
            .semantic_search(
                &session_id.to_string(),
                &[],
                self.config.long_term_top_k,
                &short_term_ids,
            )
            .iter()
            .map(|(r, sim)| TurnRecord::to_memory_chunk(r, *sim))
            .collect();
        debug!(count = long_term.len(), "长期记忆检索完成");

        // ── Step 4: 系统 Prompt ────────────────────────────────────────
        let system_prompt = SystemPrompt {
            role_description: Some(self.config.system_role.clone()),
            ..SystemPrompt::default()
        };

        // ── Step 5: Typestate 纯计算流程 ──────────────────────────────
        let step1 = chat::clean(RawInput {
            text: user_text.into(),
            turn_id,
        });
        let step3 = chat::retrieve_context(step1, short_term, long_term, system_prompt);
        let step4 = chat::assemble_prompt(
            step3,
            self.config.token_limit,
            self.config.truncation_strategy.clone(),
        );

        if step4.was_truncated {
            info!("上下文已裁剪，策略: {:?}", step4.truncation_strategy);
        }
        debug!(
            tokens = step4.total_tokens.0,
            msgs = step4.messages.len(),
            "Prompt 组装完成"
        );

        // ── Step 6: Chat 补全 ──────────────────────────────────────────
        let (raw_text, completion_tokens, stop_reason) = self
            .call_chat_api(&step4.messages)
            .await
            .context("Chat 补全失败")?;
        info!(tokens = completion_tokens.0, "模型回复完成");

        // ── Step 7: 完成类型链 ─────────────────────────────────────────
        let step5 =
            chat::inject_llm_response(step4, raw_text.clone(), completion_tokens, stop_reason);
        let answer = chat::finalize(step5);

        // ── Step 8: 记忆写回 ───────────────────────────────────────────
        let combined = format!("用户: {user_text}\n助手: {raw_text}");
        let reply_embedding = self
            .embed_text(&combined)
            .await
            .unwrap_or_else(|_| Vector(vec![]));

        self.db.append_turn(TurnRecord {
            turn_id: answer.turn_id,
            session_id: session_id.to_string(),
            user_text: user_text.into(),
            assistant_text: raw_text.clone(),
            embedding: reply_embedding.0,
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

    // ── 私有：Embedding ───────────────────────────────────────────────

    async fn embed_text(&self, text: &str) -> Result<Vector> {
        let request = CreateEmbeddingRequest {
            model: self.config.embedding_model.clone(),
            input: text.into(),
            ..Default::default()
        };
        let response = self.client.embeddings().create(request).await?;
        let vec = response
            .data
            .into_iter()
            .next()
            .map(|e| e.embedding)
            .unwrap_or_default();
        Ok(Vector(vec))
    }

    // ── 私有：Chat 补全（直接传结构化消息列表）────────────────────────

    async fn call_chat_api(
        &self,
        messages: &[ChatMessage],
    ) -> Result<(String, TokenCount, StopReason)> {
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
            .max_tokens(1024u32)
            .build()?;

        let response = self.client.chat().create(request).await?;
        let choice = response
            .choices
            .into_iter()
            .next()
            .context("模型未返回 choice")?;

        let raw_text = choice.message.content.unwrap_or_default();
        let completion_tokens = TokenCount(
            response
                .usage
                .map(|u| u.completion_tokens as usize)
                .unwrap_or(0),
        );
        let stop_reason = match choice.finish_reason {
            Some(FinishReason::Stop) => StopReason::EndOfSequence,
            Some(FinishReason::Length) => StopReason::MaxTokens,
            Some(FinishReason::ContentFilter) => StopReason::ContentFilter,
            _ => StopReason::EndOfSequence,
        };

        Ok((raw_text, completion_tokens, stop_reason))
    }
}
