//! 同步协议状态机——纯类型驱动的生命周期与协议管理，零 I/O、零副作用。
//!
//! # 协议消息体系
//!
//! | 消息 | 触发条件 | 内容 | 接收方行为 |
//! |------|---------|------|-----------|
//! | `TurnBroadcast` | 本地新轮次（实时） | 轮次完整内容 | 直接写入 DB |
//! | `StateBroadcast(Full)` | 新上线 / 邻居上线 | 全量会话水位 | 比对后 P2P 补传 |
//! | `StateBroadcast(Incremental)` | 定期超时 | 变更会话水位 | 比对后 P2P 补传 |
//! | P2P `SyncRequest` | 收到水位广播后比对 | 缺失数据请求 | 按需应答 |
//!
//! # 状态转移图
//!
//! ```text
//! SyncDisconnected
//!   → into_syncing(ticket, watermarks) → SyncSyncing
//!
//! SyncSyncing
//!   ├── handle(now, NeighborUp)           → 首次全量广播
//!   ├── handle(now, NewLocalTurn)         → 实时 TurnBroadcast
//!   ├── handle(now, RemoteTurn)           → 乱序/间隙检测 + P2P 补传
//!   ├── handle(now, RemoteState)          → 比对差异 + P2P 补传
//!   ├── handle(now, Timeout)              → 定期 Incremental 广播
//!   └── into_disconnected                 → SyncDisconnected
//! ```
//!
//! # 乱序处理
//!
//! gossip 可能乱序投递。状态机用 `BTreeSet<turn_num>` 追踪已收到的轮次，
//! `contiguous` 为最大连续前缀。收到间隙轮次时立即发起 P2P 补传填补缺口。

use std::collections::{BTreeSet, HashMap};
use std::str::FromStr;
use std::time::{Duration, Instant};

use base64ct::Encoding;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use chat_pm_session::SessionId;

use crate::device::DeviceId;
use crate::reconcile::{
    GossipMessage, InEvent, OutEvent, SessionSnapshot, SessionWatermark, StateBroadcast, StateKind,
    SyncRequest, TurnBroadcast, TurnSnapshot, compute_request,
};

// ── 常量 ────────────────────────────────────────────────────────────

/// 定期增量广播间隔。
const INCREMENTAL_INTERVAL: Duration = Duration::from_secs(30);

// ── SessionState ────────────────────────────────────────────────────

/// 状态机内部维护的单会话追踪状态。
///
/// 追踪每个会话的已知轮次集合、连续前缀、脏标记等，
/// 用于乱序检测和增量广播判断。
#[derive(Debug, Clone)]
struct SessionState {
    session_id: SessionId,
    /// 已收到的 turn_num 集合（用于乱序检测）。
    received: BTreeSet<u64>,
    /// 最大连续前缀：拥有 `1..=contiguous` 的所有轮次。
    contiguous: u64,
    has_title: bool,
    created_at: DateTime<Utc>,
    /// 自上次增量广播以来是否产生新轮次。
    dirty: bool,
}

impl SessionState {
    fn from_watermark(wm: &SessionWatermark) -> Self {
        let received: BTreeSet<u64> = (1..=wm.turn_count).collect();
        Self {
            session_id: wm.session_id,
            received,
            contiguous: wm.turn_count,
            has_title: wm.has_title,
            created_at: wm.created_at,
            dirty: true, // 初始加载视为"新"数据
        }
    }

    fn from_session_snapshot(s: &SessionSnapshot, turn_count: u64) -> Self {
        let received: BTreeSet<u64> = (1..=turn_count).collect();
        Self {
            session_id: s.session_id,
            received,
            contiguous: turn_count,
            has_title: s.title.is_some(),
            created_at: s.created_at,
            dirty: true,
        }
    }

    fn to_watermark(&self) -> SessionWatermark {
        SessionWatermark {
            session_id: self.session_id,
            turn_count: self.contiguous,
            has_title: self.has_title,
            created_at: self.created_at,
        }
    }

    /// 重新计算连续前缀。
    fn recalc_contiguous(&mut self) {
        let mut prev = 0u64;
        for &n in &self.received {
            if n == prev + 1 {
                prev = n;
            } else {
                break;
            }
        }
        self.contiguous = prev;
    }

    /// 检查接收 `turn_num` 后是否存在间隙。
    fn has_gap(&self) -> bool {
        self.received
            .last()
            .is_some_and(|max| *max > self.contiguous)
    }
}

// ── SyncConfig ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncConfig {
    pub bind_port: Option<u16>,
    pub device_name: Option<String>,
}

// ── SyncTicket ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyncTicket(distributed_topic_tracker::TopicId);

impl SyncTicket {
    pub fn new() -> Self {
        let part1 = uuid::Uuid::new_v4().to_bytes_le();
        let part2 = uuid::Uuid::new_v4().to_bytes_le();
        let mut hash: [u8; 32] = [0; 32];
        hash[..16].copy_from_slice(&part1);
        hash[16..].copy_from_slice(&part2);

        Self(distributed_topic_tracker::TopicId::from_hash(&hash))
    }
}

impl Default for SyncTicket {
    fn default() -> Self {
        Self::new()
    }
}

impl From<distributed_topic_tracker::TopicId> for SyncTicket {
    fn from(ticket: distributed_topic_tracker::TopicId) -> Self {
        Self(ticket)
    }
}
impl From<SyncTicket> for distributed_topic_tracker::TopicId {
    fn from(ticket: SyncTicket) -> Self {
        ticket.0
    }
}

impl std::fmt::Display for SyncTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = [0u8; 43];
        let encoded = base64ct::Base64UrlUnpadded::encode(&self.0.hash(), &mut buf).unwrap();
        f.write_str(encoded)
    }
}

#[derive(Debug, Clone, thiserror::Error)]
#[error("无效的同步凭证: {0}")]
pub struct SyncTicketError(String);

impl std::str::FromStr for SyncTicket {
    type Err = SyncTicketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = [0u8; 32];
        base64ct::Base64UrlUnpadded::decode(s.as_bytes(), &mut buf)
            .map_err(|e| SyncTicketError(e.to_string()))?;
        let ticket = distributed_topic_tracker::TopicId::from_hash(&buf);
        Ok(Self(ticket))
    }
}

impl Serialize for SyncTicket {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_string().serialize(s)
    }
}
impl<'de> Deserialize<'de> for SyncTicket {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = <&str>::deserialize(d)?;
        SyncTicket::from_str(s).map_err(serde::de::Error::custom)
    }
}

// ── 状态类型 ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SyncDisconnected;

#[derive(Debug, Clone)]
pub struct SyncSyncing {
    pub ticket: SyncTicket,
    /// 所有已知会话的内部状态（按 session_id 索引）。
    sessions: HashMap<SessionId, SessionState>,
    /// 下次超时的绝对时间点。`None` 表示尚未设定（NeighborUp 后首次设定）。
    next_timeout_at: Option<Instant>,
}

// ── SyncMachine ──────────────────────────────────────────────────────

pub struct SyncMachine<S> {
    state: S,
    pub device_id: DeviceId,
    pub config: SyncConfig,
}

impl SyncMachine<SyncDisconnected> {
    pub fn new(device_id: DeviceId, config: SyncConfig) -> Self {
        Self {
            state: SyncDisconnected,
            device_id,
            config,
        }
    }

    /// 进入同步状态。
    ///
    /// 调用方在拿到机器后应**立即**调用 `handle(now, NeighborUp)` 获取初始全量广播。
    pub fn into_syncing(
        self,
        ticket: SyncTicket,
        watermarks: Vec<SessionWatermark>,
    ) -> SyncMachine<SyncSyncing> {
        let sessions: HashMap<SessionId, SessionState> = watermarks
            .iter()
            .map(|wm| (wm.session_id, SessionState::from_watermark(wm)))
            .collect();

        SyncMachine {
            state: SyncSyncing {
                ticket,
                sessions,
                next_timeout_at: None,
            },
            device_id: self.device_id,
            config: self.config,
        }
    }
}

impl SyncMachine<SyncSyncing> {
    // ── 查询 ──────────────────────────────────────────────────────────

    pub fn ticket(&self) -> &SyncTicket {
        &self.state.ticket
    }

    /// 当前所有会话的水位快照（基于连续前缀）。
    pub fn watermarks(&self) -> Vec<SessionWatermark> {
        self.state
            .sessions
            .values()
            .map(|s| s.to_watermark())
            .collect()
    }

    /// 下次超时的绝对时间点。
    ///
    /// 引擎据此设定 `tokio::time::sleep_until`。返回 `None` 表示无需定时
    /// （首次 NeighborUp 之前的状态）。
    pub fn poll_timeout(&self) -> Option<Instant> {
        self.state.next_timeout_at
    }

    // ── 事件处理 ──────────────────────────────────────────────────────

    /// 处理输入事件，返回需要执行的外部动作。
    ///
    /// `now` 由调用方注入，不得在内部调用 `Instant::now()`。
    pub fn handle(&mut self, now: Instant, event: InEvent) -> impl Iterator<Item = OutEvent> + '_ {
        let outputs = match event {
            InEvent::NewLocalTurn(turn) => self.handle_new_local_turn(now, turn),
            InEvent::RemoteTurn { from_device, turn } => {
                self.handle_remote_turn(now, from_device, turn)
            }
            InEvent::RemoteState {
                from_device,
                sessions,
            } => self.handle_remote_state(from_device, &sessions),
            InEvent::RemoteSession(snapshot) => self.handle_remote_session(snapshot),
            InEvent::NeighborUp => self.handle_neighbor_up(now),
            InEvent::Leave => Vec::new(),
            InEvent::Timeout => self.handle_timeout(now),
        };
        outputs.into_iter()
    }

    // ── 生命周期 ──────────────────────────────────────────────────────

    pub fn into_disconnected(self) -> SyncMachine<SyncDisconnected> {
        SyncMachine {
            state: SyncDisconnected,
            device_id: self.device_id,
            config: self.config,
        }
    }

    // ── 内部处理函数 ──────────────────────────────────────────────────

    fn handle_new_local_turn(&mut self, _now: Instant, turn: TurnSnapshot) -> Vec<OutEvent> {
        let sid = turn.session_id;
        let turn_num = turn.turn_num;

        let state = self.get_or_create_session(sid, turn.created_at, true);

        // 检查重复
        if state.received.contains(&turn_num) {
            return Vec::new();
        }

        state.received.insert(turn_num);
        state.recalc_contiguous();
        state.dirty = true;

        vec![OutEvent::BroadcastGossip(GossipMessage::TurnBroadcast(
            TurnBroadcast {
                device_id: self.device_id,
                session_id: sid,
                turn_num,
                user_text: turn.user_text,
                assistant_text: turn.assistant_text,
                created_at: turn.created_at,
            },
        ))]
    }

    fn handle_remote_turn(
        &mut self,
        _now: Instant,
        from_device: DeviceId,
        turn: TurnSnapshot,
    ) -> Vec<OutEvent> {
        if from_device == self.device_id {
            return Vec::new();
        }

        let sid = turn.session_id;
        let turn_num = turn.turn_num;

        let state = self.get_or_create_session(sid, turn.created_at, true);
        let prev_contiguous = state.contiguous;

        if state.received.contains(&turn_num) {
            return Vec::new();
        }

        state.received.insert(turn_num);
        state.recalc_contiguous();
        state.dirty = true;

        let mut outputs = vec![OutEvent::WriteTurn(turn)];

        // 检查是否产生间隙：新 contiguous < 最大 seen
        if state.has_gap() {
            // 请求填补从 (prev_contiguous+1) 开始的缺口
            let start = prev_contiguous + 1;
            if start > 1 || prev_contiguous == 0 {
                outputs.push(OutEvent::RequestBackfill {
                    to_device: from_device,
                    request: SyncRequest {
                        need_sessions: Vec::new(),
                        need_turns: vec![(sid, start)],
                    },
                });
            }
        }

        outputs
    }

    fn handle_remote_state(
        &self,
        from_device: DeviceId,
        remote_sessions: &[SessionWatermark],
    ) -> Vec<OutEvent> {
        if from_device == self.device_id {
            return Vec::new();
        }

        let local_watermarks: Vec<SessionWatermark> = self
            .state
            .sessions
            .values()
            .map(|s| s.to_watermark())
            .collect();

        let request = compute_request(&local_watermarks, remote_sessions);

        if request.need_sessions.is_empty() && request.need_turns.is_empty() {
            return Vec::new();
        }

        vec![OutEvent::RequestBackfill {
            to_device: from_device,
            request,
        }]
    }

    fn handle_remote_session(&mut self, snapshot: SessionSnapshot) -> Vec<OutEvent> {
        if self.state.sessions.contains_key(&snapshot.session_id) {
            return Vec::new();
        }

        self.state.sessions.insert(
            snapshot.session_id,
            SessionState::from_session_snapshot(&snapshot, 0),
        );

        vec![OutEvent::WriteSession(snapshot)]
    }

    fn handle_neighbor_up(&mut self, now: Instant) -> Vec<OutEvent> {
        // 首次设定超时截止时间
        self.state.next_timeout_at = Some(now + INCREMENTAL_INTERVAL);

        let watermarks: Vec<SessionWatermark> = self
            .state
            .sessions
            .values()
            .map(|s| s.to_watermark())
            .collect();

        vec![OutEvent::BroadcastGossip(GossipMessage::StateBroadcast(
            StateBroadcast {
                device_id: self.device_id,
                kind: StateKind::Full,
                sessions: watermarks,
            },
        ))]
    }

    fn handle_timeout(&mut self, now: Instant) -> Vec<OutEvent> {
        // 推进下次超时
        self.state.next_timeout_at = Some(now + INCREMENTAL_INTERVAL);

        let dirty_watermarks: Vec<SessionWatermark> = self
            .state
            .sessions
            .values_mut()
            .filter(|s| s.dirty)
            .map(|s| {
                s.dirty = false;
                s.to_watermark()
            })
            .collect();

        if dirty_watermarks.is_empty() {
            return Vec::new();
        }

        vec![OutEvent::BroadcastGossip(GossipMessage::StateBroadcast(
            StateBroadcast {
                device_id: self.device_id,
                kind: StateKind::Incremental,
                sessions: dirty_watermarks,
            },
        ))]
    }

    /// 获取或惰性创建会话状态。
    fn get_or_create_session(
        &mut self,
        sid: SessionId,
        created_at: DateTime<Utc>,
        has_title: bool,
    ) -> &mut SessionState {
        self.state
            .sessions
            .entry(sid)
            .or_insert_with(|| SessionState {
                session_id: sid,
                received: BTreeSet::new(),
                contiguous: 0,
                has_title,
                created_at,
                dirty: true,
            })
    }
}

// ── 测试 ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn make_machine() -> SyncMachine<SyncDisconnected> {
        SyncMachine::new(DeviceId::generate(), SyncConfig::default())
    }

    fn make_test_ticket() -> SyncTicket {
        let raw: [u8; 32] = [
            0xae, 0x58, 0xff, 0x88, 0x33, 0x24, 0x1a, 0xc8, 0x2d, 0x6f, 0xf7, 0x61, 0x10, 0x46,
            0xed, 0x67, 0xb5, 0x07, 0x2d, 0x14, 0x2c, 0x58, 0x8d, 0x00, 0x63, 0xe9, 0x42, 0xd9,
            0xa7, 0x55, 0x02, 0xb6,
        ];
        let ticket = distributed_topic_tracker::TopicId::from_hash(&raw);
        SyncTicket::from(ticket)
    }

    fn make_watermarks() -> Vec<SessionWatermark> {
        vec![SessionWatermark {
            session_id: SessionId::new(),
            turn_count: 1,
            has_title: true,
            created_at: Utc::now(),
        }]
    }

    fn make_turn(session_id: SessionId, turn_num: u64, device_id: DeviceId) -> TurnSnapshot {
        TurnSnapshot {
            turn_id: chat_pm_session::TurnId::generate(),
            session_id,
            turn_num,
            user_text: format!("user_{turn_num}"),
            assistant_text: format!("asst_{turn_num}"),
            created_at: Utc::now(),
            device_id,
        }
    }

    #[test]
    fn test_ticket_roundtrip() {
        let ticket = make_test_ticket();
        let s = ticket.to_string();
        let parsed = SyncTicket::from_str(&s).unwrap();
        assert_eq!(ticket.to_string(), parsed.to_string());
    }

    #[test]
    fn test_into_syncing_builds_sessions() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let sid = watermarks[0].session_id;

        let machine = machine.into_syncing(ticket.clone(), watermarks.clone());

        assert_eq!(machine.ticket().to_string(), ticket.to_string());
        let wms = machine.watermarks();
        assert_eq!(wms.len(), 1);
        assert_eq!(wms[0].session_id, sid);
        assert_eq!(wms[0].turn_count, 1);
    }

    #[test]
    fn test_neighbor_up_returns_full_broadcast() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let did = machine.device_id;

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        let outputs: Vec<OutEvent> = machine.handle(now, InEvent::NeighborUp).collect();

        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            OutEvent::BroadcastGossip(GossipMessage::StateBroadcast(sb)) => {
                assert_eq!(sb.device_id, did);
                assert_eq!(sb.kind, StateKind::Full);
                assert_eq!(sb.sessions.len(), 1);
            }
            other => panic!("expected StateBroadcast, got {other:?}"),
        }

        // poll_timeout should be set after NeighborUp
        assert!(machine.poll_timeout().is_some());
    }

    #[test]
    fn test_new_local_turn_returns_turn_broadcast() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let sid = watermarks[0].session_id;

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        let turn = make_turn(sid, 2, machine.device_id);
        let outputs: Vec<OutEvent> = machine.handle(now, InEvent::NewLocalTurn(turn)).collect();

        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            OutEvent::BroadcastGossip(GossipMessage::TurnBroadcast(tb)) => {
                assert_eq!(tb.session_id, sid);
                assert_eq!(tb.turn_num, 2);
            }
            other => panic!("expected TurnBroadcast, got {other:?}"),
        }
    }

    #[test]
    fn test_remote_turn_normal_append() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let sid = watermarks[0].session_id;
        let remote_device = DeviceId::generate();

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        // turn_num=2: contiguous=1, so this is normal append (1+1=2)
        let turn = make_turn(sid, 2, remote_device);
        let outputs: Vec<OutEvent> = machine
            .handle(
                now,
                InEvent::RemoteTurn {
                    from_device: remote_device,
                    turn,
                },
            )
            .collect();

        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            OutEvent::WriteTurn(_) => {}
            other => panic!("expected WriteTurn, got {other:?}"),
        }
    }

    #[test]
    fn test_remote_turn_gap_triggers_backfill() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let sid = watermarks[0].session_id;
        let remote_device = DeviceId::generate();

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        // turn_num=5, but contiguous=1 → gap! Need 2,3,4
        let turn = make_turn(sid, 5, remote_device);
        let outputs: Vec<OutEvent> = machine
            .handle(
                now,
                InEvent::RemoteTurn {
                    from_device: remote_device,
                    turn,
                },
            )
            .collect();

        assert_eq!(outputs.len(), 2);
        let has_write = outputs.iter().any(|o| matches!(o, OutEvent::WriteTurn(_)));
        let has_backfill = outputs
            .iter()
            .any(|o| matches!(o, OutEvent::RequestBackfill { .. }));
        assert!(has_write);
        assert!(has_backfill);
    }

    #[test]
    fn test_remote_turn_duplicate_ignored() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let sid = watermarks[0].session_id;
        let remote_device = DeviceId::generate();

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        // turn_num=1: already has it (from watermarks)
        let turn = make_turn(sid, 1, remote_device);
        let outputs: Vec<OutEvent> = machine
            .handle(
                now,
                InEvent::RemoteTurn {
                    from_device: remote_device,
                    turn,
                },
            )
            .collect();

        assert!(outputs.is_empty());
    }

    #[test]
    fn test_remote_state_triggers_backfill_when_missing() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let existing_sid = watermarks[0].session_id;
        let remote_device = DeviceId::generate();

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        // Remote has a session we don't have
        let new_sid = SessionId::new();
        let remote_sessions = vec![
            SessionWatermark {
                session_id: existing_sid,
                turn_count: 3, // we have 1, remote has 3 → missing turns 2,3
                has_title: true,
                created_at: Utc::now(),
            },
            SessionWatermark {
                session_id: new_sid,
                turn_count: 1,
                has_title: false,
                created_at: Utc::now(),
            },
        ];

        let outputs: Vec<OutEvent> = machine
            .handle(
                now,
                InEvent::RemoteState {
                    from_device: remote_device,
                    sessions: remote_sessions,
                },
            )
            .collect();

        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            OutEvent::RequestBackfill { to_device, request } => {
                assert_eq!(*to_device, remote_device);
                assert_eq!(request.need_sessions.len(), 1); // new_sid
                assert_eq!(request.need_turns.len(), 1); // (existing_sid, 2)
            }
            other => panic!("expected RequestBackfill, got {other:?}"),
        }
    }

    #[test]
    fn test_timeout_broadcasts_dirty_sessions() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let did = machine.device_id;

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        // First, NeighborUp to set the timeout clock
        let _ = machine.handle(now, InEvent::NeighborUp).collect::<Vec<_>>();

        // Timeout with dirty sessions (all are dirty initially)
        let timeout_now = now + INCREMENTAL_INTERVAL;
        let outputs: Vec<OutEvent> = machine.handle(timeout_now, InEvent::Timeout).collect();

        assert_eq!(outputs.len(), 1);
        match &outputs[0] {
            OutEvent::BroadcastGossip(GossipMessage::StateBroadcast(sb)) => {
                assert_eq!(sb.device_id, did);
                assert_eq!(sb.kind, StateKind::Incremental);
                assert_eq!(sb.sessions.len(), 1);
            }
            other => panic!("expected StateBroadcast(Incremental), got {other:?}"),
        }

        // Timeout again with no dirty sessions → no broadcast
        let timeout_now2 = timeout_now + INCREMENTAL_INTERVAL;
        let outputs2: Vec<OutEvent> = machine.handle(timeout_now2, InEvent::Timeout).collect();
        assert!(outputs2.is_empty());
    }

    #[test]
    fn test_leave_returns_empty() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();

        let mut machine = machine.into_syncing(ticket, watermarks);
        let now = Instant::now();

        let outputs: Vec<OutEvent> = machine.handle(now, InEvent::Leave).collect();
        assert!(outputs.is_empty());
    }

    #[test]
    fn test_into_disconnected() {
        let machine = make_machine();
        let machine = machine.into_syncing(make_test_ticket(), make_watermarks());
        let _ = machine.into_disconnected();
    }
}
