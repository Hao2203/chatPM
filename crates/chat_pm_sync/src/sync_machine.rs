//! 同步状态机——纯类型驱动的生命周期管理，零 I/O、零副作用。
//!
//! # 状态转移图
//!
//! ```text
//! SyncDisconnected
//!   → into_syncing(ticket, watermarks) → (SyncSyncing, SyncAnnouncement)
//! SyncSyncing → into_disconnected → SyncDisconnected
//! ```
//!
//! `into_syncing` 将当前水位组装为 [`SyncAnnouncement`] 一并返回，
//! 调用方拿到后**必须**将 Announcement 广播——编译期无中间态但返回值强制提醒。

use std::str::FromStr;

use base64ct::Encoding;
use distributed_topic_tracker::TopicId;
use serde::{Deserialize, Serialize};

use crate::device::DeviceId;
use crate::reconcile::{SessionWatermark, SyncAnnouncement};

// ── SyncConfig ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncConfig {
    pub bind_port: Option<u16>,
    pub device_name: Option<String>,
}

// ── SyncTicket ──────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SyncTicket(TopicId);

impl SyncTicket {
    pub fn new() -> Self {
        let part1 = uuid::Uuid::new_v4().to_bytes_le();
        let part2 = uuid::Uuid::new_v4().to_bytes_le();
        let mut hash: [u8; 32] = [0; 32];
        hash[..16].copy_from_slice(&part1);
        hash[16..].copy_from_slice(&part2);

        Self(TopicId::from_hash(&hash))
    }
}

impl Default for SyncTicket {
    fn default() -> Self {
        Self::new()
    }
}

impl From<TopicId> for SyncTicket {
    fn from(ticket: TopicId) -> Self {
        Self(ticket)
    }
}
impl From<SyncTicket> for TopicId {
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

impl FromStr for SyncTicket {
    type Err = SyncTicketError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut buf = [0u8; 32];
        base64ct::Base64UrlUnpadded::decode(s.as_bytes(), &mut buf)
            .map_err(|e| SyncTicketError(e.to_string()))?;
        let ticket = TopicId::from_hash(&buf);
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

    /// 进入同步状态，并[]将当前水位组装为 [`SyncAnnouncement`] 一并返回。
    ///
    /// 调用方**必须**拿到 Announcement 后广播。
    pub fn into_syncing(
        self,
        ticket: SyncTicket,
        watermarks: Vec<SessionWatermark>,
    ) -> (SyncMachine<SyncSyncing>, SyncAnnouncement) {
        let announcement = SyncAnnouncement {
            device_id: self.device_id,
            sessions: watermarks,
        };
        let machine = SyncMachine {
            state: SyncSyncing { ticket },
            device_id: self.device_id,
            config: self.config,
        };
        (machine, announcement)
    }
}

impl SyncMachine<SyncSyncing> {
    pub fn ticket(&self) -> &SyncTicket {
        &self.state.ticket
    }

    pub fn into_disconnected(self) -> SyncMachine<SyncDisconnected> {
        SyncMachine {
            state: SyncDisconnected,
            device_id: self.device_id,
            config: self.config,
        }
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
        let ticket = TopicId::from_hash(&raw);
        SyncTicket::from(ticket)
    }

    fn make_watermarks() -> Vec<SessionWatermark> {
        vec![SessionWatermark {
            session_id: chat_pm_session::SessionId::new(),
            turn_count: 1,
            has_title: true,
            created_at: chrono::Utc::now(),
        }]
    }

    #[test]
    fn test_ticket_roundtrip() {
        let ticket = make_test_ticket();
        let s = ticket.to_string();
        let parsed = SyncTicket::from_str(&s).unwrap();
        assert_eq!(ticket.to_string(), parsed.to_string());
    }

    #[test]
    fn test_into_syncing_returns_announcement() {
        let machine = make_machine();
        let ticket = make_test_ticket();
        let watermarks = make_watermarks();
        let did = machine.device_id;

        let (machine, announcement) = machine.into_syncing(ticket.clone(), watermarks.clone());

        assert_eq!(machine.ticket().to_string(), ticket.to_string());
        assert_eq!(announcement.device_id, did);
        assert_eq!(announcement.sessions.len(), 1);
        assert_eq!(
            announcement.sessions[0].session_id.to_string(),
            watermarks[0].session_id.to_string()
        );
    }

    #[test]
    fn test_into_disconnected() {
        let machine = make_machine();
        let (machine, _) = machine.into_syncing(make_test_ticket(), make_watermarks());
        let _ = machine.into_disconnected();
    }
}
