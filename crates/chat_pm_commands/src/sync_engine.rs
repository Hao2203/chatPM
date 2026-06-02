//! 同步引擎——基于 iroh 的 P2P 设备间会话同步。
//!
//! `SyncEngine` 是非泛型 I/O 容器。纯类型状态机由
//! `chat_pm_sync::sync_machine::SyncMachine<S>` 提供，`SyncEngine` 在
//! init 过程中驱动状态转换，完成后始终处于 Syncing 态。
//!
//! init 完成后立即广播自身状态；邻居上线时通过 [`events()`] 通道通知上层。

use std::{sync::Arc, time::Duration};

use anyhow;
use chat_pm_database::{ChatDb, DbError};
use chat_pm_sync::{
    DeviceId, SyncAnnouncement, SyncError, SyncPayload, SyncRequest, compute_sync_request,
    parse_sync_payload,
    sync_machine::{SyncMachine, SyncSyncing},
};
use distributed_topic_tracker::{
    AutoDiscoveryGossip, BootstrapConfig, DhtConfig, RecordPublisher, TopicId,
};
use ed25519_dalek::SigningKey;
use iroh::{Endpoint, EndpointId, SecretKey, address_lookup::mdns, endpoint::presets};
use iroh_gossip::{api::Event as GossipEvent, net::Gossip};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tracing::{error, info, warn};

// ── 重导出 ──────────────────────────────────────────────────────────

pub use chat_pm_sync::sync_machine::{SyncConfig, SyncTicket};

// ── SyncEngineError ──────────────────────────────────────────────────

#[derive(Debug, thiserror::Error)]
pub enum SyncEngineError {
    #[error("[Sync Error] {0}")]
    Sync(#[from] SyncError),
    #[error("[Database Error] {0}")]
    Db(#[from] DbError),
    #[error("[Network Error] {0:#}")]
    Network(anyhow::Error),
    #[error("[Serialization Error] {0}")]
    Serialization(#[from] serde_json::Error),
    #[error(
        "[Protocol Error] invalid sync message format (received {:?})",
        received
    )]
    InvalidSyncFormat { received: Vec<u8> },
    #[error(
        "[Protocol Error] incomplete sync message (expected {expected} bytes, got {actual} bytes)"
    )]
    IncompleteSyncMessage { expected: usize, actual: usize },
    #[error("[Protocol Error] response missing payload (message: {msg_text})")]
    MissingPayload { msg_text: String },
    #[error("[Protocol Error] request missing SyncRequest (message: {msg_text})")]
    MissingRequest { msg_text: String },
    #[error("[Internal Error] database lock poisoned")]
    DatabaseLock,
    #[error("[Internal Error] {0:#}")]
    Internal(anyhow::Error),
}

pub type SyncEngineResult<T> = Result<T, SyncEngineError>;

// ── 常量 ────────────────────────────────────────────────────────────

const SYNC_ALPN: &[u8] = b"/chatpm/sync-req/1";
const SYNC_PROTO_MAGIC: &[u8; 4] = b"cSPM";

// ── 内部 I/O 资源 ───────────────────────────────────────────────────

#[allow(dead_code)]
struct NetRes {
    endpoint: Endpoint,
    gossip: Gossip,
    signing_key: SigningKey,
}

#[allow(dead_code)]
struct DocRes {
    topic: distributed_topic_tracker::Topic,
}

// ── 事件 ────────────────────────────────────────────────────────────

/// 同步引擎事件。
#[derive(Debug, Clone)]
pub enum SyncEvent {
    /// 邻居上线，建议调用 `publish_announcement` 广播自身状态
    NeighborUp,
}

// ── SyncEngine ──────────────────────────────────────────────────────

/// 同步引擎——非泛型，init 后始终处于 Syncing 状态。
pub struct SyncEngine {
    machine: SyncMachine<SyncSyncing>,
    net: NetRes,
    doc: DocRes,
    db: Arc<std::sync::Mutex<ChatDb>>,
    event_tx: broadcast::Sender<SyncEvent>,
    _bg: BackgroundSyncHandle,
}

impl SyncEngine {
    /// 创建新同步链——创建文档后立即广播自身状态。
    pub async fn create(
        db: Arc<std::sync::Mutex<ChatDb>>,
        config: SyncConfig,
        device_id: DeviceId,
        secret_key: Option<[u8; 32]>,
    ) -> SyncEngineResult<Self> {
        do_init(db, config, device_id, secret_key, None).await
    }

    /// 凭 ticket 加入已有同步链——加入后立即广播自身状态。
    pub async fn join(
        db: Arc<std::sync::Mutex<ChatDb>>,
        config: SyncConfig,
        device_id: DeviceId,
        secret_key: Option<[u8; 32]>,
        ticket: SyncTicket,
    ) -> SyncEngineResult<Self> {
        do_init(db, config, device_id, secret_key, Some(ticket)).await
    }

    pub fn ticket(&self) -> &SyncTicket {
        self.machine.ticket()
    }

    pub fn device_id(&self) -> DeviceId {
        self.machine.device_id
    }

    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.net.signing_key.to_bytes()
    }

    /// 广播自身状态（创建/恢复时由 init 自动调用，邻居上线后上层主动调用）。
    pub async fn publish_announcement(&self) -> SyncEngineResult<()> {
        broadcast_current_state(&self.db, self.machine.device_id, &self.doc.topic).await
    }

    /// 返回事件接收器（邻居上线等）。
    pub fn events(&self) -> broadcast::Receiver<SyncEvent> {
        self.event_tx.subscribe()
    }
}

// ── 内部 init ───────────────────────────────────────────────────────

async fn do_init(
    db: Arc<std::sync::Mutex<ChatDb>>,
    config: SyncConfig,
    device_id: DeviceId,
    secret_key: Option<[u8; 32]>,
    ticket: Option<SyncTicket>,
) -> SyncEngineResult<SyncEngine> {
    // 1. 初始化网络
    let secret_key = match secret_key {
        Some(bytes) => SecretKey::from_bytes(&bytes),
        None => SecretKey::generate(),
    };
    let signing_key = SigningKey::from_bytes(&secret_key.to_bytes());

    let mdns = mdns::MdnsAddressLookup::builder();
    let endpoint = Endpoint::builder(presets::N0)
        .secret_key(secret_key)
        .alpns(vec![SYNC_ALPN.to_vec(), iroh_gossip::ALPN.to_vec()])
        .address_lookup(mdns)
        .bind()
        .await
        .map_err(sync_net_err)?;

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
        .accept(SYNC_ALPN, SyncHandler { db: db.clone() })
        .spawn();

    info!(node_id = %endpoint.id(), "同步引擎网络已初始化");

    let net = NetRes {
        endpoint: endpoint.clone(),
        gossip: gossip.clone(),
        signing_key: signing_key.clone(),
    };

    // 2. 初始化 topic
    let sync_ticket = if let Some(v) = ticket {
        v
    } else {
        SyncTicket::new()
    };
    let topic = init_topic(sync_ticket.clone().into(), gossip.clone(), signing_key).await?;

    info!("同步已启动");

    // 3. 构建水位 → 转入同步态（拿到需广播的 Announcement）
    let watermarks = {
        let guard = lock_db(&db)?;
        guard.build_watermarks(device_id)?
    };
    let (machine, announcement) =
        SyncMachine::new(device_id, config).into_syncing(sync_ticket, watermarks);

    // 4. 广播
    let json = serde_json::to_vec(&announcement)?;
    let sender = topic.gossip_sender().await.map_err(sync_net_err)?;
    sender.broadcast(json).await.map_err(sync_net_err)?;
    info!(sessions = announcement.sessions.len(), "初始广播已发送");

    // 5. 事件通道
    let (event_tx, _) = broadcast::channel::<SyncEvent>(16);

    let topic_for_bg = topic.clone();

    // 6. 启动后台监听
    let bg = spawn_background(
        db.clone(),
        device_id,
        endpoint,
        topic_for_bg,
        event_tx.clone(),
    );

    Ok(SyncEngine {
        machine,
        net,
        doc: DocRes { topic },
        db,
        event_tx,
        _bg: bg,
    })
}

// ── 广播辅助 ────────────────────────────────────────────────────────

async fn broadcast_current_state(
    db: &Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    topic: &distributed_topic_tracker::Topic,
) -> SyncEngineResult<()> {
    let watermarks = {
        let guard = lock_db(db)?;
        guard.build_watermarks(device_id)?
    };

    let announcement = SyncAnnouncement {
        device_id,
        sessions: watermarks,
    };

    let json = serde_json::to_vec(&announcement)?;
    let sender = topic.gossip_sender().await.map_err(sync_net_err)?;
    sender.broadcast(json).await.map_err(sync_net_err)?;

    info!(device_id = %device_id, sessions = announcement.sessions.len(), "同步公告已发布");
    Ok(())
}

// ── BackgroundSyncHandle ────────────────────────────────────────────

pub struct BackgroundSyncHandle {
    cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl BackgroundSyncHandle {
    pub fn cancel(mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for BackgroundSyncHandle {
    fn drop(&mut self) {
        if let Some(tx) = self.cancel.take() {
            let _ = tx.send(());
        }
    }
}

fn spawn_background(
    db: Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    endpoint: Endpoint,
    topic: distributed_topic_tracker::Topic,
    event_tx: broadcast::Sender<SyncEvent>,
) -> BackgroundSyncHandle {
    let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

    tokio::spawn(async move {
        tokio::select! {
            _ = cancel_rx => info!("后台同步已取消"),
            _ = run_sync_loop(db, device_id, endpoint, topic, event_tx) => {
                warn!("后台同步循环已退出");
            }
        }
    });

    BackgroundSyncHandle {
        cancel: Some(cancel_tx),
    }
}

// ── 内部辅助 ────────────────────────────────────────────────────────

async fn init_topic(
    topic_id: TopicId,
    gossip: Gossip,
    signing_key: SigningKey,
) -> SyncEngineResult<distributed_topic_tracker::Topic> {
    let initial_secret = b"chatpm-sync-initial".to_vec();
    let config = distributed_topic_tracker::Config::builder()
        .dht_config(
            DhtConfig::builder()
                .retries(3)
                .base_retry_interval(Duration::from_secs(5))
                .max_retry_jitter(Duration::from_secs(10))
                .get_timeout(Duration::from_secs(10))
                .put_timeout(Duration::from_secs(10))
                .build(),
        )
        .bootstrap_config(
            BootstrapConfig::builder()
                .max_bootstrap_records(2)
                .publish_record_on_startup(true)
                .check_older_records_first_on_startup(true)
                .discovery_poll_interval(Duration::from_secs(5))
                .no_peers_retry_interval(Duration::from_secs(5))
                .per_peer_join_settle_time(Duration::from_millis(500))
                .join_confirmation_wait_time(Duration::from_millis(5000))
                .build(),
        )
        .build();
    let record_publisher =
        RecordPublisher::new(topic_id, signing_key, None, initial_secret, config);

    let topic = gossip
        .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
        .await
        .map_err(sync_net_err)?;

    Ok(topic)
}

async fn run_sync_loop(
    db: Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    endpoint: Endpoint,
    topic: distributed_topic_tracker::Topic,
    event_tx: broadcast::Sender<SyncEvent>,
) {
    let Ok(mut receiver) = topic.gossip_receiver().await else {
        error!("无法获取 gossip receiver");
        return;
    };
    info!("后台同步循环已启动");

    loop {
        match receiver.next().await {
            Ok(GossipEvent::Received(msg)) => {
                match serde_json::from_slice::<SyncAnnouncement>(&msg.content) {
                    Ok(announcement) => {
                        if let Err(e) = process_remote_announcement(
                            &db,
                            device_id,
                            &endpoint,
                            msg.delivered_from,
                            announcement,
                        )
                        .await
                        {
                            error!(%e, "处理远程公告失败");
                        }
                    }
                    Err(e) => warn!(%e, "解析远程公告失败"),
                }
            }
            Ok(GossipEvent::NeighborUp(peer)) => {
                info!(%peer, "邻居上线——通知上层广播");
                let _ = event_tx.send(SyncEvent::NeighborUp);
            }
            Ok(GossipEvent::NeighborDown(peer)) => info!(%peer, "邻居下线"),
            Ok(GossipEvent::Lagged) => warn!("gossip 消息滞后"),
            Err(e) => {
                error!(%e, "gossip receiver 错误");
                break;
            }
        }
    }
}

async fn process_remote_announcement(
    db: &Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    endpoint: &Endpoint,
    from_peer: EndpointId,
    announcement: SyncAnnouncement,
) -> SyncEngineResult<()> {
    if announcement.device_id == device_id {
        return Ok(());
    }
    info!(
        from_device = %announcement.device_id,
        sessions = announcement.sessions.len(),
        "收到远程同步公告"
    );

    let local_watermarks = {
        let guard = lock_db(db)?;
        guard.build_watermarks(device_id)?
    };

    let request = compute_sync_request(&local_watermarks, &announcement);
    if request.need_sessions.is_empty() && request.need_turns.is_empty() {
        return Ok(());
    }
    info!(
        need_sessions = request.need_sessions.len(),
        need_turns = request.need_turns.len(),
        "需要同步数据"
    );

    let payload = request_sync(endpoint, from_peer, &request).await?;
    let verified = parse_sync_payload(payload)?;

    let guard = lock_db(db)?;
    let count = guard.apply_verified_payload(&verified)?;
    info!(turns_written = count, "同步数据已写入本地数据库");
    Ok(())
}

// ── 传输层 ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMessage {
    request: Option<SyncRequest>,
    payload: Option<SyncPayload>,
}

impl SyncMessage {
    /// 编码为线格式：魔数 + 4 字节大端长度 + JSON。
    fn to_wire(&self) -> SyncEngineResult<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        let mut data = SYNC_PROTO_MAGIC.to_vec();
        data.extend_from_slice(&(json.len() as u32).to_be_bytes());
        data.extend_from_slice(&json);
        Ok(data)
    }

    /// 从线格式解码。校验魔数和长度前缀。
    fn from_wire(buf: &[u8]) -> SyncEngineResult<Self> {
        if buf.len() < 8 || &buf[..4] != SYNC_PROTO_MAGIC {
            let received = buf[..buf.len().min(64)].to_vec();
            return Err(SyncEngineError::InvalidSyncFormat { received });
        }
        let json_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
        if buf.len() < 8 + json_len {
            return Err(SyncEngineError::IncompleteSyncMessage {
                expected: 8 + json_len,
                actual: buf.len(),
            });
        }
        let msg: Self = serde_json::from_slice(&buf[8..8 + json_len])?;
        Ok(msg)
    }

    fn into_request(self) -> SyncEngineResult<SyncRequest> {
        let msg_text =
            serde_json::to_string(&self).unwrap_or_else(|e| format!("(serialization failed: {e})"));
        self.request
            .ok_or(SyncEngineError::MissingRequest { msg_text })
    }

    fn into_payload(self) -> SyncEngineResult<SyncPayload> {
        let msg_text =
            serde_json::to_string(&self).unwrap_or_else(|e| format!("(serialization failed: {e})"));
        self.payload
            .ok_or(SyncEngineError::MissingPayload { msg_text })
    }
}

/// 锁定数据库，PoisonError 转 `DatabaseLock`。
fn lock_db(
    db: &Arc<std::sync::Mutex<ChatDb>>,
) -> SyncEngineResult<std::sync::MutexGuard<'_, ChatDb>> {
    db.lock().map_err(|_| SyncEngineError::DatabaseLock)
}

async fn request_sync(
    endpoint: &Endpoint,
    peer: EndpointId,
    request: &SyncRequest,
) -> SyncEngineResult<SyncPayload> {
    let conn = endpoint
        .connect(peer, SYNC_ALPN)
        .await
        .map_err(sync_net_err)?;

    let msg = SyncMessage {
        request: Some(request.clone()),
        payload: None,
    };
    let send_data = msg.to_wire()?;

    let (mut send, mut recv) = conn.open_bi().await.map_err(sync_net_err)?;
    send.write_all(&send_data).await.map_err(sync_net_err)?;
    send.finish()
        .map_err(|e| SyncEngineError::Network(e.into()))?;

    let buf = recv.read_to_end(1024 * 1024).await.map_err(sync_net_err)?;
    let response = SyncMessage::from_wire(&buf)?;
    let payload = response.into_payload()?;
    Ok(payload)
}

fn sync_net_err(e: impl Into<anyhow::Error>) -> SyncEngineError {
    SyncEngineError::Network(e.into())
}

#[derive(Debug, Clone)]
struct SyncHandler {
    db: Arc<std::sync::Mutex<ChatDb>>,
}

impl iroh::protocol::ProtocolHandler for SyncHandler {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let db = self.db.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_sync_connection(db, connection).await {
                warn!(%e, "处理同步连接失败");
            }
        });
        Ok(())
    }
}

/// 处理入站同步连接：读取 SyncRequest → 查询数据库组装 SyncPayload → 写回响应。
async fn handle_sync_connection(
    db: Arc<std::sync::Mutex<ChatDb>>,
    connection: iroh::endpoint::Connection,
) -> SyncEngineResult<()> {
    let (mut send, mut recv) = connection.accept_bi().await.map_err(sync_net_err)?;

    // 读取请求
    let buf = recv.read_to_end(1024 * 1024).await.map_err(sync_net_err)?;
    let msg = SyncMessage::from_wire(&buf)?;
    let request = msg.into_request()?;

    info!(
        need_sessions = request.need_sessions.len(),
        need_turns = request.need_turns.len(),
        "收到同步请求"
    );

    // 组装响应
    let payload = build_sync_payload(&db, &request)?;

    info!(
        sessions = payload.sessions.len(),
        turns = payload.turns.len(),
        "组装同步响应"
    );

    // 写回响应
    let response = SyncMessage {
        request: None,
        payload: Some(payload),
    };
    let send_data = response.to_wire()?;

    send.write_all(&send_data).await.map_err(sync_net_err)?;
    send.finish()
        .map_err(|e| SyncEngineError::Network(e.into()))?;

    info!("同步响应已发送");
    Ok(())
}

/// 根据 SyncRequest 从数据库查询并组装 SyncPayload。
fn build_sync_payload(
    db: &Arc<std::sync::Mutex<ChatDb>>,
    request: &SyncRequest,
) -> SyncEngineResult<SyncPayload> {
    let guard = lock_db(db)?;

    let mut sessions = Vec::new();
    let mut turns = Vec::new();

    // 请求完整会话：包含会话快照 + 所有轮次
    for session_id in &request.need_sessions {
        if let Some(snapshot) = guard.get_session_snapshot(*session_id)? {
            sessions.push(snapshot);
            let session_turns = guard.get_turns_from(*session_id, 1)?;
            turns.extend(session_turns);
        }
    }

    // 请求增量轮次：仅获取指定起始位置之后的轮次
    for (session_id, start_turn) in &request.need_turns {
        let session_turns = guard.get_turns_from(*session_id, *start_turn)?;
        turns.extend(session_turns);
    }

    Ok(SyncPayload { sessions, turns })
}
