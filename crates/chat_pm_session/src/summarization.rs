use crate::TurnId;

#[derive(Debug, Clone)]
pub struct Summary {
    pub content: String,
    pub last_turn_id: TurnId,
    pub last_turn_num: u64,
}

/// 判断 prompt_tokens 是否超过阈值，需要触发摘要压缩。
pub fn should_summarize(prompt_tokens: usize, context_window: usize, ratio: f64) -> bool {
    if context_window == 0 || ratio <= 0.0 {
        return false;
    }
    (prompt_tokens as f64) > (context_window as f64 * ratio)
}

/// 摘要规划结果。
#[derive(Debug, Clone)]
pub struct SummarizationPlan {
    /// 新摘要应覆盖到的最后一轮编号（含）。
    pub new_last_turn_num: u64,
}

/// 规划摘要范围。
///
/// 返回 `None` 表示当前无需压缩（总轮数在短期窗口内，或没有新的轮次需要纳入摘要）。
pub fn plan_summarization(
    total_turns: u64,
    short_term_turns: usize,
    existing_summary: Option<&Summary>,
) -> Option<SummarizationPlan> {
    let short_term = short_term_turns as u64;
    if total_turns <= short_term {
        return None; // 总轮数尚未超过短期窗口
    }

    let cutoff = total_turns - short_term;
    let last_summarized = existing_summary.map(|s| s.last_turn_num).unwrap_or(0);

    if cutoff <= last_summarized {
        return None; // 没有新的轮次需要纳入摘要
    }

    Some(SummarizationPlan {
        new_last_turn_num: cutoff,
    })
}

/// 返回需要从 DB 查询的轮次范围 `(from, to)`（包含两端）。
///
/// - `from`: 上次摘要覆盖的下一轮（1-based），若无摘要则为 1。
/// - `to`: 规划中指定的截止轮。
pub fn turn_range_to_summarize(
    existing_summary: Option<&Summary>,
    plan: &SummarizationPlan,
) -> (u64, u64) {
    let from = existing_summary.map(|s| s.last_turn_num + 1).unwrap_or(1);
    (from, plan.new_last_turn_num)
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid;

    #[test]
    fn test_should_summarize_below_threshold() {
        assert!(!should_summarize(500_000, 1_000_000, 0.6));
    }

    #[test]
    fn test_should_summarize_above_threshold() {
        assert!(should_summarize(650_000, 1_000_000, 0.6));
    }

    #[test]
    fn test_plan_summarization_too_few_turns() {
        assert!(plan_summarization(5, 6, None).is_none());
    }

    #[test]
    fn test_plan_summarization_first_time() {
        let plan = plan_summarization(10, 6, None).unwrap();
        assert_eq!(plan.new_last_turn_num, 4);
    }

    #[test]
    fn test_plan_summarization_incremental() {
        let existing = Summary {
            content: "old summary".into(),
            last_turn_id: crate::TurnId::from_uuid(uuid::Uuid::nil()),
            last_turn_num: 4,
        };
        let plan = plan_summarization(12, 6, Some(&existing)).unwrap();
        assert_eq!(plan.new_last_turn_num, 6);
    }

    #[test]
    fn test_plan_summarization_no_new_turns() {
        let existing = Summary {
            content: "old summary".into(),
            last_turn_id: crate::TurnId::from_uuid(uuid::Uuid::nil()),
            last_turn_num: 6,
        };
        // total=12, short=6, cutoff=6, last_summarized=6 → cutoff <= last
        assert!(plan_summarization(12, 6, Some(&existing)).is_none());
    }

    #[test]
    fn test_turn_range_first_time() {
        let plan = SummarizationPlan {
            new_last_turn_num: 4,
        };
        let (from, to) = turn_range_to_summarize(None, &plan);
        assert_eq!(from, 1);
        assert_eq!(to, 4);
    }

    #[test]
    fn test_turn_range_incremental() {
        let existing = Summary {
            content: "old".into(),
            last_turn_id: crate::TurnId::from_uuid(uuid::Uuid::nil()),
            last_turn_num: 4,
        };
        let plan = SummarizationPlan {
            new_last_turn_num: 6,
        };
        let (from, to) = turn_range_to_summarize(Some(&existing), &plan);
        assert_eq!(from, 5);
        assert_eq!(to, 6);
    }
}
