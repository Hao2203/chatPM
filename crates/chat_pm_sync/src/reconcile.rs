//! 会话同步的核心协调逻辑。
//!
//! 纯函数实现跨设备同步所需的数据类型和协调算法，不涉及任何 I/O 或网络细节。
//! 调用方（gossip 层或命令层）提供数据，本模块计算差异并验证数据一致性。
//!
//! # 同步流程
//!
//! 1. **通告（Announcement）**: 本设备变更后，广播 [`SyncAnnouncement`] 描述所知会话水位
//! 2. **比对（Reconcile）**: 接收方调用 [`compute_sync_request`] 计算缺失数据
//! 3. **请求（Request）**: 接收方向发送方发起 [`SyncRequest`]
//! 4. **响应（Payload）**: 发送方组装 [`SyncPayload`] 返回实际数据
//! 5. **解析（Parse）**: 接收方调用 [`parse_sync_payload`] 消费原始负载，
//!    返回 [`VerifiedPayload`]——携带结构一致性证明的类型

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use chat_pm_session::{SessionId, TurnId};

use crate::device::DeviceId;

// ── Watermark ──────────────────────────────────────────────────────

/// 设备对某个会话的知识水位。
///
/// 描述某台设备在一个会话中所知的数据量，用于在 gossip 网络中
/// 传播本设备的状态摘要。接收方通过比对双方水位来判断是否需要同步。
///
/// `turn_count` 是本设备在该会话中已知的轮次总数。由于轮次是 append-only，
/// 轮次数可作为可靠的同步进度指标。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionWatermark {
    pub session_id: SessionId,
    pub turn_count: u64,
    pub has_title: bool,
    pub created_at: DateTime<Utc>,
}

// ── Announcement ───────────────────────────────────────────────────

/// 本设备在 gossip 网络中发布的同步声明。
///
/// 包含本设备所知的所有会话水位。接收方将自身水位与声明比对，
/// 决定是否需要发起同步请求。
///
/// 序列化后体积小，适合广播传播。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncAnnouncement {
    pub device_id: DeviceId,
    pub sessions: Vec<SessionWatermark>,
}

// ── Request ────────────────────────────────────────────────────────

/// 一个设备向另一个设备发起的同步请求。
///
/// 描述请求方在本设备上缺失的数据条目，接收方据此组装响应负载。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRequest {
    /// 请求方完全缺失的会话 ID 列表。
    /// 接收方需要提供这些会话的完整信息（含所有轮次）。
    pub need_sessions: Vec<SessionId>,

    /// 请求方部分缺失的轮次，值为 `(session_id, start_turn_num)`。
    /// 表示需要从 `start_turn_num` 开始往后的所有轮次。
    /// 例如 `(sess_1, 5)` 表示需要 sess_1 中 turn_num >= 5 的所有轮次。
    pub need_turns: Vec<(SessionId, u64)>,
}

// ── Payload ────────────────────────────────────────────────────────

/// 同步响应负载，包含实际的会话和轮次数据。
///
/// 当一方收到另一方发来的 [`SyncRequest`] 后，从本地数据库查询数据并组装此负载返回。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPayload {
    pub sessions: Vec<SessionSnapshot>,
    pub turns: Vec<TurnSnapshot>,
}

/// 会话快照，用于跨设备同步。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub session_id: SessionId,
    pub created_at: DateTime<Utc>,
    pub title: Option<String>,
}

/// 轮次快照，用于跨设备同步。
///
/// 携带产生该轮次的原始设备 ID，以便接收方追踪数据来源。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSnapshot {
    pub turn_id: TurnId,
    pub session_id: SessionId,
    pub turn_num: u64,
    pub user_text: String,
    pub assistant_text: String,
    pub created_at: DateTime<Utc>,
    pub device_id: DeviceId,
}

// ── Reconciliation ────────────────────────────────────────────────

/// 根据本地会话水表和远程通告，计算需要向对方请求哪些数据。
///
/// # 同步规则
///
/// | 本地有？ | 对方有？ | 轮次数比较 | 行为 |
/// |---|---|---|---|
/// | 否 | 是 | — | 全量请求该会话（`need_sessions`） |
/// | 是 | 是 | 对方更多 | 请求缺失轮次（`need_turns`） |
/// | 是 | 是 | 相同或本地更多 | 无需操作 |
/// | 是 | 否 | — | 无需操作（本地领先） |
///
/// # 与 Chrono / SQLite 的集成
///
/// 调用方在构造 [`SessionWatermark`] 时，可将数据库中的 `created_at`
/// （Unix 时间戳）通过 `DateTime::from_timestamp` 转换为 `DateTime<Utc>`：
///
/// ```ignore
/// use chrono::{DateTime, Utc};
/// let dt = DateTime::from_timestamp(created_at_secs, 0).unwrap();
/// ```
pub fn compute_sync_request(local: &[SessionWatermark], remote: &SyncAnnouncement) -> SyncRequest {
    let local_map: HashMap<SessionId, &SessionWatermark> =
        local.iter().map(|w| (w.session_id, w)).collect();

    let mut need_sessions = Vec::new();
    let mut need_turns = Vec::new();

    for remote_ws in &remote.sessions {
        match local_map.get(&remote_ws.session_id) {
            None => {
                // 对方有这个会话，本地没有 → 全量请求
                need_sessions.push(remote_ws.session_id);
            }
            Some(local_ws) => {
                // 双方都有此会话 → 按轮次数补齐
                if remote_ws.turn_count > local_ws.turn_count {
                    let start_turn = local_ws.turn_count + 1;
                    need_turns.push((remote_ws.session_id, start_turn));
                }
            }
        }
    }

    SyncRequest {
        need_sessions,
        need_turns,
    }
}

/// 经过结构一致性验证的同步负载。
///
/// 此类型的唯一构造方式是通过 [`parse_sync_payload`]，一旦构造成功则保证：
///
/// **不变量：**
/// - 所有轮次的 `session_id` 在会话列表中都有对应项（无孤儿轮次）
/// - 会话快照中没有重复的 `session_id`
/// - 轮次快照中没有重复的 `turn_id`
///
/// 消费此类型的函数无需重复检查这些约束——类型系统已在构造时证明。
#[derive(Debug, Clone)]
pub struct VerifiedPayload {
    sessions: Vec<SessionSnapshot>,
    turns: Vec<TurnSnapshot>,
    /// 会话 ID 索引，提供 O(1) 的成员查询
    session_index: HashSet<SessionId>,
}

impl VerifiedPayload {
    /// 返回所有已验证的会话快照。
    pub fn sessions(&self) -> &[SessionSnapshot] {
        &self.sessions
    }

    /// 返回所有已验证的轮次快照。
    pub fn turns(&self) -> &[TurnSnapshot] {
        &self.turns
    }

    /// 消费自身，返回内部数据。
    pub fn into_inner(self) -> (Vec<SessionSnapshot>, Vec<TurnSnapshot>) {
        (self.sessions, self.turns)
    }

    /// 检查指定的会话 ID 是否存在于负载中。
    pub fn contains_session(&self, session_id: SessionId) -> bool {
        self.session_index.contains(&session_id)
    }
}

/// 解析原始同步负载，返回携带结构一致性证明的 [`VerifiedPayload`]。
///
/// 这是消费 [`SyncPayload`] 的唯一合法入口。一旦解析成功，返回的 [`VerifiedPayload`]
/// 保证满足所有结构不变量，消费者无需重复检查。
///
/// # 解析规则
///
/// | 检查项 | 不通过则返回 |
/// |---|---|
/// | 会话 `session_id` 无重复 | `SyncError::DuplicateSession` |
/// | 轮次 `turn_id` 无重复 | `SyncError::DuplicateTurn` |
/// | 轮次的 `session_id` 在会话列表中存在 | `SyncError::OrphanedTurn` |
pub fn parse_sync_payload(payload: SyncPayload) -> Result<VerifiedPayload, SyncError> {
    let SyncPayload { sessions, turns } = payload;

    // 构建会话索引，同时检查重复
    let mut session_index = HashSet::new();
    for s in &sessions {
        if !session_index.insert(s.session_id) {
            return Err(SyncError::DuplicateSession(s.session_id));
        }
    }

    // 检查轮次引用有效且无重复 turn_id
    let mut turn_ids = HashSet::new();
    for t in &turns {
        if !session_index.contains(&t.session_id) {
            return Err(SyncError::OrphanedTurn(t.session_id));
        }
        if !turn_ids.insert(t.turn_id) {
            return Err(SyncError::DuplicateTurn(t.turn_id));
        }
    }

    Ok(VerifiedPayload {
        sessions,
        turns,
        session_index,
    })
}

// ── Error ──────────────────────────────────────────────────────────

/// 同步处理过程中可能出现的错误。
///
/// 所有变体均为结构校验错误，不含 I/O 类错误（I/O 错误由调用方处理）。
#[derive(Debug, Clone, thiserror::Error)]
pub enum SyncError {
    /// 同步数据中出现了轮次，但其所属会话不在会话列表中
    #[error("同步数据中发现孤儿轮次：session_id={0} 不存在于会话列表中")]
    OrphanedTurn(SessionId),

    /// 同步数据中存在重复的会话 ID
    #[error("同步数据中存在重复的会话：{0}")]
    DuplicateSession(SessionId),

    /// 同步数据中存在重复的轮次 ID
    #[error("同步数据中存在重复的轮次：{0}")]
    DuplicateTurn(TurnId),
}

// ── Tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_watermark(session_id: SessionId, turn_count: u64, has_title: bool) -> SessionWatermark {
        SessionWatermark {
            session_id,
            turn_count,
            has_title,
            created_at: Utc::now(),
        }
    }

    fn make_dummy_announcement(sessions: Vec<SessionWatermark>) -> SyncAnnouncement {
        SyncAnnouncement {
            device_id: DeviceId::generate(),
            sessions,
        }
    }

    // ── compute_sync_request ──────────────────────────────────────

    #[test]
    fn test_request_nothing_when_up_to_date() {
        let sid = SessionId::new();
        let local = [make_watermark(sid, 5, true)];
        let remote = make_dummy_announcement(vec![make_watermark(sid, 5, true)]);

        let req = compute_sync_request(&local, &remote);
        assert!(req.need_sessions.is_empty());
        assert!(req.need_turns.is_empty());
    }

    #[test]
    fn test_request_nothing_when_local_ahead() {
        let sid = SessionId::new();
        let local = [make_watermark(sid, 10, true)];
        let remote = make_dummy_announcement(vec![make_watermark(sid, 5, true)]);

        let req = compute_sync_request(&local, &remote);
        assert!(req.need_sessions.is_empty());
        assert!(req.need_turns.is_empty());
    }

    #[test]
    fn test_request_new_sessions() {
        let sid1 = SessionId::new();
        let sid2 = SessionId::new();
        let local = [make_watermark(sid1, 3, true)];
        let remote = make_dummy_announcement(vec![
            make_watermark(sid1, 3, true),
            make_watermark(sid2, 2, false),
        ]);

        let req = compute_sync_request(&local, &remote);
        assert_eq!(req.need_sessions, vec![sid2]);
        assert!(req.need_turns.is_empty());
    }

    #[test]
    fn test_request_missing_turns() {
        let sid = SessionId::new();
        let local = [make_watermark(sid, 3, true)];
        let remote = make_dummy_announcement(vec![make_watermark(sid, 7, true)]);

        let req = compute_sync_request(&local, &remote);
        assert!(req.need_sessions.is_empty());
        assert_eq!(req.need_turns, vec![(sid, 4)]);
    }

    #[test]
    fn test_request_sessions_and_turns() {
        let sid_common = SessionId::new();
        let sid_remote = SessionId::new();

        let local = [make_watermark(sid_common, 2, true)];
        let remote = make_dummy_announcement(vec![
            make_watermark(sid_common, 5, true),
            make_watermark(sid_remote, 3, false),
        ]);

        let req = compute_sync_request(&local, &remote);
        assert_eq!(req.need_sessions, vec![sid_remote]);
        assert_eq!(req.need_turns, vec![(sid_common, 3)]);
    }

    #[test]
    fn test_request_empty_local_needs_everything() {
        let sid = SessionId::new();
        let local: [SessionWatermark; 0] = [];
        let remote = make_dummy_announcement(vec![make_watermark(sid, 5, true)]);

        let req = compute_sync_request(&local, &remote);
        assert_eq!(req.need_sessions, vec![sid]);
        assert!(req.need_turns.is_empty());
    }

    #[test]
    fn test_request_remote_has_no_sessions() {
        let sid = SessionId::new();
        let local = [make_watermark(sid, 5, true)];
        let remote = make_dummy_announcement(vec![]);

        let req = compute_sync_request(&local, &remote);
        assert!(req.need_sessions.is_empty());
        assert!(req.need_turns.is_empty());
    }

    // ── parse_sync_payload ────────────────────────────────────────

    fn dummy_session(session_id: SessionId) -> SessionSnapshot {
        SessionSnapshot {
            session_id,
            created_at: Utc::now(),
            title: None,
        }
    }

    fn dummy_turn(turn_id: TurnId, session_id: SessionId, turn_num: u64) -> TurnSnapshot {
        TurnSnapshot {
            turn_id,
            session_id,
            turn_num,
            user_text: format!("user_{}", turn_num),
            assistant_text: format!("asst_{}", turn_num),
            created_at: Utc::now(),
            device_id: DeviceId::generate(),
        }
    }

    #[test]
    fn test_parse_empty_payload() {
        let payload = SyncPayload {
            sessions: vec![],
            turns: vec![],
        };
        let parsed = parse_sync_payload(payload).unwrap();
        assert!(parsed.sessions().is_empty());
        assert!(parsed.turns().is_empty());
    }

    #[test]
    fn test_parse_valid_payload() {
        let sid = SessionId::new();
        let payload = SyncPayload {
            sessions: vec![dummy_session(sid)],
            turns: vec![dummy_turn(TurnId::generate(), sid, 1)],
        };
        let parsed = parse_sync_payload(payload).unwrap();
        assert_eq!(parsed.sessions().len(), 1);
        assert_eq!(parsed.turns().len(), 1);
        assert!(parsed.contains_session(sid));
    }

    #[test]
    fn test_parse_multiple_sessions_and_turns() {
        let s1 = SessionId::new();
        let s2 = SessionId::new();
        let payload = SyncPayload {
            sessions: vec![dummy_session(s1), dummy_session(s2)],
            turns: vec![
                dummy_turn(TurnId::generate(), s1, 1),
                dummy_turn(TurnId::generate(), s1, 2),
                dummy_turn(TurnId::generate(), s2, 1),
            ],
        };
        let parsed = parse_sync_payload(payload).unwrap();
        assert_eq!(parsed.sessions().len(), 2);
        assert_eq!(parsed.turns().len(), 3);
        assert!(parsed.contains_session(s1));
        assert!(parsed.contains_session(s2));
    }

    #[test]
    fn test_parse_orphaned_turn() {
        let sid = SessionId::new();
        let orphan_sid = SessionId::new();
        let payload = SyncPayload {
            sessions: vec![dummy_session(sid)],
            turns: vec![dummy_turn(TurnId::generate(), orphan_sid, 1)],
        };
        assert!(matches!(
            parse_sync_payload(payload),
            Err(SyncError::OrphanedTurn(_))
        ));
    }

    #[test]
    fn test_parse_duplicate_session() {
        let sid = SessionId::new();
        let payload = SyncPayload {
            sessions: vec![dummy_session(sid), dummy_session(sid)],
            turns: vec![],
        };
        assert!(matches!(
            parse_sync_payload(payload),
            Err(SyncError::DuplicateSession(_))
        ));
    }

    #[test]
    fn test_parse_duplicate_turn() {
        let sid = SessionId::new();
        let tid = TurnId::generate();
        let payload = SyncPayload {
            sessions: vec![dummy_session(sid)],
            turns: vec![dummy_turn(tid, sid, 1), dummy_turn(tid, sid, 2)],
        };
        assert!(matches!(
            parse_sync_payload(payload),
            Err(SyncError::DuplicateTurn(_))
        ));
    }

    #[test]
    fn test_verified_payload_into_inner() {
        let sid = SessionId::new();
        let payload = SyncPayload {
            sessions: vec![dummy_session(sid)],
            turns: vec![dummy_turn(TurnId::generate(), sid, 1)],
        };
        let parsed = parse_sync_payload(payload).unwrap();
        let (sessions, turns) = parsed.into_inner();
        assert_eq!(sessions.len(), 1);
        assert_eq!(turns.len(), 1);
    }
}
