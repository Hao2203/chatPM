//! # 上下文感知 Chat 流程 —— 状态类型建模
//!
//! 流程：
//!   RawInput
//!     → CleanedInput        (文本清洗)
//!     → EmbeddedQuery       (向量化)
//!     → RetrievedContext    (拼装短期 + 长期记忆)
//!     → AssembledPrompt     (组装最终 Prompt，含 token 裁剪)
//!     → LlmResponse         (模型推理结果)
//!     → FinalAnswer         (后处理 + 记忆更新计划)

use crate::language::Language;

#[derive(Debug, Clone)]
pub struct Vector(pub Vec<f32>);

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Similarity(pub f32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TokenCount(pub usize);

impl TokenCount {
    pub fn exceeds(&self, limit: TokenCount) -> bool {
        self.0 > limit.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnId(pub u64);

// ─────────────────────────────────────────
// 结构化消息（对应 OpenAI messages 数组的一条）
// ─────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
    pub fn token_count(&self) -> TokenCount {
        estimate_tokens(&self.content)
    }
}

// ─────────────────────────────────────────
// 记忆片段
// ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MemoryChunk {
    pub turn: TurnId,
    /// 用户发言
    pub user_text: String,
    /// 助手回复
    pub assistant_text: String,
    pub relevance: Similarity,
}

impl MemoryChunk {
    pub fn token_count(&self) -> TokenCount {
        TokenCount(estimate_tokens(&self.user_text).0 + estimate_tokens(&self.assistant_text).0)
    }
    /// 展开为一对 ChatMessage（user + assistant）
    pub fn to_messages(&self) -> [ChatMessage; 2] {
        [
            ChatMessage::user(self.user_text.clone()),
            ChatMessage::assistant(self.assistant_text.clone()),
        ]
    }
}

// ─────────────────────────────────────────
// 系统提示
// ─────────────────────────────────────────

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

    pub fn token_count(&self) -> TokenCount {
        estimate_tokens(&self.to_message().content)
    }
}

// ─────────────────────────────────────────
// ══ 状态类型定义 ══
// ─────────────────────────────────────────

#[derive(Debug)]
pub struct RawInput {
    pub text: String,
    pub turn_id: TurnId,
}

#[derive(Debug)]
pub struct CleanedInput {
    pub text: String,
    pub turn_id: TurnId,
}

#[derive(Debug)]
pub struct EmbeddedQuery {
    pub text: String,
    pub turn_id: TurnId,
    pub query_vector: Vector,
}

#[derive(Debug)]
pub struct RetrievedContext {
    pub text: String,
    pub turn_id: TurnId,
    pub short_term: Vec<MemoryChunk>,
    pub long_term: Vec<MemoryChunk>,
    pub system_prompt: SystemPrompt,
}

// ── 裁剪策略 ────────────────────────────

#[derive(Debug, Clone)]
pub enum TruncationStrategy {
    /// 优先保留相关度高的片段
    ByRelevance,
    /// 优先保留最新的片段
    ByRecency,
    /// 先裁长期记忆，再裁短期记忆
    LongTermFirst,
}

// ── 阶段 4：结构化消息列表 ───────────────
//
// messages 顺序：
//   [system]
//   [user, assistant] × 历史轮次（按时间升序）
//   [user]            ← 当前输入，永远在最后

#[derive(Debug)]
pub struct AssembledPrompt {
    pub turn_id: TurnId,
    pub messages: Vec<ChatMessage>,
    pub total_tokens: TokenCount,
    pub was_truncated: bool,
    pub truncation_strategy: Option<TruncationStrategy>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StopReason {
    EndOfSequence,
    MaxTokens,
    ContentFilter,
}

#[derive(Debug)]
pub struct LlmResponse {
    pub turn_id: TurnId,
    pub raw_text: String,
    pub prompt_tokens: TokenCount,
    pub completion_tokens: TokenCount,
    pub stop_reason: StopReason,
}

#[derive(Debug)]
pub struct MemoryUpdatePlan {
    pub content_to_store: String,
    pub turn_id: TurnId,
}

#[derive(Debug)]
pub struct FinalAnswer {
    pub turn_id: TurnId,
    pub display_text: String,
    pub truncation_warning: Option<String>,
    pub memory_update_plan: MemoryUpdatePlan,
}

// ─────────────────────────────────────────
// ══ 状态转换函数 ══
// ─────────────────────────────────────────

pub fn clean(input: RawInput) -> CleanedInput {
    CleanedInput {
        text: clean_text(&input.text),
        turn_id: input.turn_id,
    }
}

pub fn retrieve_context(
    query: CleanedInput,
    short_term: Vec<MemoryChunk>,
    long_term: Vec<MemoryChunk>,
    system_prompt: SystemPrompt,
) -> RetrievedContext {
    let mut long_term_sorted = long_term;
    long_term_sorted.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    RetrievedContext {
        text: query.text,
        turn_id: query.turn_id,
        short_term,
        long_term: long_term_sorted,
        system_prompt,
    }
}

pub fn assemble_prompt(
    ctx: RetrievedContext,
    token_limit: TokenCount,
    strategy: TruncationStrategy,
) -> AssembledPrompt {
    let system_msg = ctx.system_prompt.to_message();
    let current_msg = ChatMessage::user(ctx.text.clone());
    let reserved = TokenCount(system_msg.token_count().0 + current_msg.token_count().0);
    let available = TokenCount(token_limit.0.saturating_sub(reserved.0));

    let (selected_chunks, was_truncated) =
        select_chunks_within_budget(&ctx.short_term, &ctx.long_term, available, &strategy);

    // 组装消息列表：system → 历史(user+assistant 交替) → 当前 user
    let mut messages = Vec::new();
    messages.push(system_msg);
    for chunk in &selected_chunks {
        let [u, a] = chunk.to_messages();
        messages.push(u);
        messages.push(a);
    }
    messages.push(current_msg);

    let total_tokens = TokenCount(messages.iter().map(|m| m.token_count().0).sum());

    AssembledPrompt {
        turn_id: ctx.turn_id,
        messages,
        total_tokens,
        was_truncated,
        truncation_strategy: if was_truncated { Some(strategy) } else { None },
    }
}

pub fn inject_llm_response(
    prompt: AssembledPrompt,
    raw_text: String,
    completion_tokens: TokenCount,
    stop_reason: StopReason,
) -> LlmResponse {
    LlmResponse {
        turn_id: prompt.turn_id,
        prompt_tokens: prompt.total_tokens,
        raw_text,
        completion_tokens,
        stop_reason,
    }
}

pub fn finalize(response: LlmResponse) -> FinalAnswer {
    let truncation_warning = if response.stop_reason == StopReason::MaxTokens {
        Some("回答因长度限制被截断，请尝试更具体的问题。".to_string())
    } else {
        None
    };

    FinalAnswer {
        turn_id: response.turn_id,
        display_text: response.raw_text.clone(),
        truncation_warning,
        memory_update_plan: MemoryUpdatePlan {
            content_to_store: response.raw_text,
            turn_id: response.turn_id,
        },
    }
}

// ─────────────────────────────────────────
// 内部纯函数
// ─────────────────────────────────────────

fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn estimate_tokens(text: &str) -> TokenCount {
    TokenCount(text.chars().count() / 2 + 1)
}

fn select_chunks_within_budget<'a>(
    short_term: &'a [MemoryChunk],
    long_term: &'a [MemoryChunk],
    budget: TokenCount,
    strategy: &TruncationStrategy,
) -> (Vec<&'a MemoryChunk>, bool) {
    let mut selected: Vec<&MemoryChunk> = Vec::new();
    let mut used = TokenCount(0);
    let mut truncated = false;

    let ordered: Vec<&MemoryChunk> = match strategy {
        TruncationStrategy::LongTermFirst => long_term.iter().chain(short_term.iter()).collect(),
        TruncationStrategy::ByRelevance => {
            let mut all: Vec<&MemoryChunk> = short_term.iter().chain(long_term.iter()).collect();
            all.sort_by(|a, b| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            all
        }
        TruncationStrategy::ByRecency => {
            let mut all: Vec<&MemoryChunk> = short_term.iter().chain(long_term.iter()).collect();
            all.sort_by_key(|b| std::cmp::Reverse(b.turn));
            all
        }
    };

    for chunk in ordered {
        let chunk_tokens = chunk.token_count();
        let new_total = TokenCount(used.0 + chunk_tokens.0);
        if new_total.exceeds(budget) {
            truncated = true;
            continue;
        }
        used = new_total;
        selected.push(chunk);
    }

    (selected, truncated)
}

// ─────────────────────────────────────────
// 测试
// ─────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chunk(turn: u64, user: &str, assistant: &str, relevance: f32) -> MemoryChunk {
        MemoryChunk {
            turn: TurnId(turn),
            user_text: user.to_string(),
            assistant_text: assistant.to_string(),
            relevance: Similarity(relevance),
        }
    }

    #[test]
    fn test_message_structure() {
        let step1 = clean(RawInput {
            text: "  Type State 模式如何应用？  ".to_string(),
            turn_id: TurnId(5),
        });
        assert_eq!(step1.text, "Type State 模式如何应用？");

        let step3 = retrieve_context(
            step1,
            vec![make_chunk(3, "什么是借用？", "借用是…", 1.0)],
            vec![make_chunk(1, "什么是所有权？", "所有权是…", 0.87)],
            SystemPrompt {
                reply_language: Some(Language::English),
                role_description: None,
            },
        );
        let step4 = assemble_prompt(step3, TokenCount(2048), TruncationStrategy::ByRelevance);

        // 验证消息结构：system + 2×(user+assistant) + user
        assert!(!step4.was_truncated);
        assert_eq!(step4.messages[0].role, Role::System);
        assert!(step4.messages[0].content.contains(Language::English.code())); // persona 已合并
        assert_eq!(step4.messages.last().unwrap().role, Role::User);
        assert_eq!(
            step4.messages.last().unwrap().content,
            "Type State 模式如何应用？"
        );

        // 历史轮次交替 user/assistant
        let history = &step4.messages[1..step4.messages.len() - 1];
        assert!(history.iter().step_by(2).all(|m| m.role == Role::User));
        assert!(
            history
                .iter()
                .skip(1)
                .step_by(2)
                .all(|m| m.role == Role::Assistant)
        );
    }

    #[test]
    fn test_token_truncation() {
        // system_prompt token_count ≈ estimate_tokens("助手") = 3
        // current_input "测试" → tokens = 2
        // reserved = 3 + 2 = 5
        // available = 11 - 5 = 6
        //
        // short chunk: user="短期" → 2, assistant="内容" → 3, total = 5; relevance=1.0
        // long  chunk: user="长期内容很长很长" → 5, assistant="很长很长很长" → 7, total=12; relevance=0.9
        //
        // ByRelevance: short(5) 先选，used=5 ≤ 6 ✓
        //              long(12): 5+12=17 > 6 → 跳过，was_truncated=true ✓
        let ctx = RetrievedContext {
            text: "测试".to_string(),
            turn_id: TurnId(1),
            short_term: vec![make_chunk(1, "短期", "内容", 1.0)],
            long_term: vec![make_chunk(0, "长期内容很长很长", "很长很长很长", 0.9)],
            system_prompt: SystemPrompt::default(),
        };
        let prompt = assemble_prompt(ctx, TokenCount(11), TruncationStrategy::ByRelevance);
        assert!(prompt.was_truncated);
    }
}
