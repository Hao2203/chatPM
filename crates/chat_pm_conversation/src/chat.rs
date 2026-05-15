#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TurnId(pub u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Role {
    System,
    User,
    Assistant,
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
    pub completion_tokens: usize,
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
// 测试
// ─────────────────────────────────────────

// #[cfg(test)]
// mod tests {
//     use super::*;

//     fn make_chunk(turn: u64, user: &str, assistant: &str, relevance: f32) -> MemoryChunk {
//         MemoryChunk {
//             turn: TurnId(turn),
//             user_text: user.to_string(),
//             assistant_text: assistant.to_string(),
//             relevance: Similarity(relevance),
//         }
//     }

//     #[test]
//     fn test_message_structure() {
//         let step1 = clean(RawInput {
//             text: "  Type State 模式如何应用？  ".to_string(),
//             turn_id: TurnId(5),
//         });
//         assert_eq!(step1.text, "Type State 模式如何应用？");

//         let step3 = retrieve_context(
//             step1,
//             vec![make_chunk(3, "什么是借用？", "借用是…", 1.0)],
//             vec![make_chunk(1, "什么是所有权？", "所有权是…", 0.87)],
//             SystemPrompt {
//                 reply_language: Some(Language::English),
//                 role_description: None,
//             },
//         );
//         let step4 = assemble_prompt(step3, TokenCount(2048), TruncationStrategy::ByRelevance);

//         // 验证消息结构：system + 2×(user+assistant) + user
//         assert!(!step4.was_truncated);
//         assert_eq!(step4.messages[0].role, Role::System);
//         assert!(step4.messages[0].content.contains(Language::English.code())); // persona 已合并
//         assert_eq!(step4.messages.last().unwrap().role, Role::User);
//         assert_eq!(
//             step4.messages.last().unwrap().content,
//             "Type State 模式如何应用？"
//         );

//         // 历史轮次交替 user/assistant
//         let history = &step4.messages[1..step4.messages.len() - 1];
//         assert!(history.iter().step_by(2).all(|m| m.role == Role::User));
//         assert!(
//             history
//                 .iter()
//                 .skip(1)
//                 .step_by(2)
//                 .all(|m| m.role == Role::Assistant)
//         );
//     }

//     #[test]
//     fn test_token_truncation() {
//         // system_prompt token_count ≈ estimate_tokens("助手") = 3
//         // current_input "测试" → tokens = 2
//         // reserved = 3 + 2 = 5
//         // available = 11 - 5 = 6
//         //
//         // short chunk: user="短期" → 2, assistant="内容" → 3, total = 5; relevance=1.0
//         // long  chunk: user="长期内容很长很长" → 5, assistant="很长很长很长" → 7, total=12; relevance=0.9
//         //
//         // ByRelevance: short(5) 先选，used=5 ≤ 6 ✓
//         //              long(12): 5+12=17 > 6 → 跳过，was_truncated=true ✓
//         let ctx = RetrievedContext {
//             text: "测试".to_string(),
//             turn_id: TurnId(1),
//             short_term: vec![make_chunk(1, "短期", "内容", 1.0)],
//             long_term: vec![make_chunk(0, "长期内容很长很长", "很长很长很长", 0.9)],
//             system_prompt: SystemPrompt::default(),
//         };
//         let prompt = assemble_prompt(ctx, TokenCount(11), TruncationStrategy::ByRelevance);
//         assert!(prompt.was_truncated);
//     }
// }
