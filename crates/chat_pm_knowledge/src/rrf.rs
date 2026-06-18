use crate::vector_store::SearchResult;

/// 使用 Reciprocal Rank Fusion 融合两个排序列表。
///
/// RRF 公式：`score(d) = Σ 1 / (k + rank_i(d))`
///
/// 其中 `k = 60`（默认值），`rank_i(d)` 是从 1 开始的排名。
/// 如果文档同时出现在两个列表中，分数相加。
/// 返回融合后按分数降序排列的结果。
///
/// # 参数
/// - `ranked_a`: 第一个排序列表（BM25 或向量搜索结果）
/// - `ranked_b`: 第二个排序列表
/// - `k`: 平滑常数，默认 60
///
/// # 返回
/// 融合后的排序结果，去除了重复的 `chunk_id`。
pub fn rrf_fuse(ranked_a: &[SearchResult], ranked_b: &[SearchResult], k: f64) -> Vec<SearchResult> {
    use std::collections::HashMap;

    // 用 chunk_id 去重并累积 RRF 分数
    let mut score_map: HashMap<String, (f64, &SearchResult)> = HashMap::new();

    // 处理第一个列表
    for (rank, result) in ranked_a.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank as f64 + 1.0));
        score_map
            .entry(result.chunk_id.clone())
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result));
    }

    // 处理第二个列表
    for (rank, result) in ranked_b.iter().enumerate() {
        let rrf_score = 1.0 / (k + (rank as f64 + 1.0));
        score_map
            .entry(result.chunk_id.clone())
            .and_modify(|(score, _)| *score += rrf_score)
            .or_insert((rrf_score, result));
    }

    // 按 RRF 分数降序排序
    let mut fused: Vec<SearchResult> = score_map
        .into_values()
        .map(|(rrf_score, result)| {
            let mut r = result.clone();
            r.score = rrf_score as f32;
            r
        })
        .collect();

    fused.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    fused
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_result(chunk_id: &str, score: f32) -> SearchResult {
        SearchResult {
            chunk_id: chunk_id.to_string(),
            document_id: "doc1".to_string(),
            chunk_index: 0,
            content: format!("content of {}", chunk_id),
            score,
        }
    }

    #[test]
    fn rrf_fuse_empty_b_returns_a() {
        let a = vec![
            make_result("a", 0.9),
            make_result("b", 0.8),
            make_result("c", 0.7),
        ];
        let result = rrf_fuse(&a, &[], 60.0);
        assert_eq!(result.len(), 3);
        // 第一个元素排名最高（分数最高）
        assert_eq!(result[0].chunk_id, "a");
    }

    #[test]
    fn rrf_fuse_empty_a_returns_b() {
        let b = vec![
            make_result("x", 0.9),
            make_result("y", 0.8),
        ];
        let result = rrf_fuse(&[], &b, 60.0);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].chunk_id, "x");
    }

    #[test]
    fn rrf_fuse_both_empty_returns_empty() {
        let result = rrf_fuse(&[], &[], 60.0);
        assert!(result.is_empty());
    }

    #[test]
    fn rrf_fuse_deduplicates_by_chunk_id() {
        let a = vec![make_result("shared", 0.9)];
        let b = vec![make_result("shared", 0.5)];
        let result = rrf_fuse(&a, &b, 60.0);
        // "shared" 在两边都出现，应该只保留一个，分数为两个 RRF 之和
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].chunk_id, "shared");
        // RRF(rank=1, k=60) = 1/61 ≈ 0.01639, 两个列表各一次 = 2*(1/61) ≈ 0.03279
        let expected = 2.0 * (1.0 / 61.0);
        assert!((result[0].score - expected as f32).abs() < 0.001,
            "expected {}, got {}", expected, result[0].score);
    }

    #[test]
    fn rrf_fuse_mixed_overlap() {
        let a = vec![
            make_result("only_a", 0.9),
            make_result("shared", 0.8),
        ];
        let b = vec![
            make_result("shared", 0.6),
            make_result("only_b", 0.5),
        ];
        let result = rrf_fuse(&a, &b, 60.0);
        // 应该有三个唯一结果
        assert_eq!(result.len(), 3);
        // "shared" 因双重排名分数最高
        assert_eq!(result[0].chunk_id, "shared");
    }

    #[test]
    fn rrf_fuse_same_list_equals_original_order() {
        let a = vec![
            make_result("x", 0.9),
            make_result("y", 0.8),
            make_result("z", 0.7),
        ];
        // 两个相同列表融合 → 每个条目 RRF 分数翻倍，但相对顺序不变
        let result = rrf_fuse(&a, &a, 60.0);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].chunk_id, "x");
        assert_eq!(result[1].chunk_id, "y");
        assert_eq!(result[2].chunk_id, "z");
    }

    #[test]
    fn rrf_k_parameter_affects_scores() {
        let a = vec![make_result("item", 1.0)];
        let b = vec![make_result("item", 1.0)];

        let with_small_k = rrf_fuse(&a, &b, 1.0);
        let with_large_k = rrf_fuse(&a, &b, 100.0);

        // smaller k → higher RRF score
        assert!(with_small_k[0].score > with_large_k[0].score,
            "k=1 score {} should be > k=100 score {}",
            with_small_k[0].score, with_large_k[0].score);
    }
}
