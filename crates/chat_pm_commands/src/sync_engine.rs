//! 同步引擎——基于 iroh 的 P2P 设备间会话同步。
//!
//! `SyncEngine` 是 I/O 容器，持有网络资源和输入通道。
//! 纯协议状态机 [`chat_pm_sync::SyncMachine`] 在后台事件循环中运行，
//! 所有事件通过 `handle_new_turn` 等方法注入。
//!
//! # 事件循环
//!
//! 后台循环统一处理三类输入：
//!
//! 1. **gossip 消息** — 解析为 [`InEvent::RemoteTurn`] 或 [`InEvent::RemoteState`]，喂入状态机。
//! 2. **外部输入** — 通过 `mpsc` 通道注入（`NewLocalTurn` 等），由 Tauri 层调用。
//! 3. **定期超时** — 由 [`SyncMachine::poll_timeout`] 驱动，触发增量水位广播。
//!
//! 状态机产出的 [`OutEvent`] 由 `dispatch` 函数分流执行。

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow;
use chat_pm_database::{ChatDb, DbError, SessionRecord};
use chat_pm_sync::{
    DeviceId, GossipMessage, InEvent, OutEvent, SyncError, SyncPayload, SyncRequest, TurnSnapshot,
    parse_sync_payload,
    sync_machine::{SyncMachine, SyncSyncing},
};
use distributed_topic_tracker::{
    AutoDiscoveryGossip, BootstrapConfig, DhtConfig, GossipReceiver, RecordPublisher, TopicId,
};
use ed25519_dalek::SigningKey;
use iroh::{Endpoint, EndpointId, SecretKey, address_lookup::mdns, endpoint::presets};
use iroh_gossip::{api::Event as GossipEvent, net::Gossip};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
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

// ── DeviceId ↔ EndpointId 不可失败转换 ──────────────────────────────

/// `DeviceId` → `EndpointId` 转换。
///
/// `DeviceId` 派生自 ed25519 公钥，故转换不会失败。
fn device_to_endpoint(device_id: DeviceId) -> EndpointId {
    EndpointId::from_bytes(&device_id.into_inner())
        .expect("DeviceId must be a valid ed25519 public key")
}

/// `EndpointId` → `DeviceId` 转换。
#[allow(dead_code)]
fn endpoint_to_device(endpoint_id: &EndpointId) -> DeviceId {
    DeviceId::from_bytes(*endpoint_id.as_bytes())
}

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

// ── SyncEngine ──────────────────────────────────────────────────────

/// 同步引擎——非泛型 I/O 容器。
///
/// 协议逻辑由内部 `SyncMachine<SyncSyncing>` 驱动，
/// 引擎提供网络 I/O 和对外 API。
#[allow(dead_code)]
pub struct SyncEngine {
    net: NetRes,
    doc: DocRes,
    db: Arc<std::sync::Mutex<ChatDb>>,
    /// 向后台事件循环注入事件的通道。
    input_tx: mpsc::UnboundedSender<(Instant, InEvent)>,
    ticket: SyncTicket,
    device_id: DeviceId,
    secret_key: [u8; 32],
    _bg: BackgroundSyncHandle,
}

impl SyncEngine {
    /// 创建新同步链。
    pub async fn create(
        db: Arc<std::sync::Mutex<ChatDb>>,
        config: SyncConfig,
        secret_key: Option<[u8; 32]>,
    ) -> SyncEngineResult<Self> {
        do_init(db, config, secret_key, None).await
    }

    /// 凭 ticket 加入已有同步链。
    pub async fn join(
        db: Arc<std::sync::Mutex<ChatDb>>,
        config: SyncConfig,
        secret_key: Option<[u8; 32]>,
        ticket: SyncTicket,
    ) -> SyncEngineResult<Self> {
        do_init(db, config, secret_key, Some(ticket)).await
    }

    /// 通知引擎本地产生了新轮次（`send_message` 完成后调用）。
    ///
    /// 通过内部通道异步注入状态机，非阻塞。
    pub fn handle_new_turn(&self, now: Instant, turn: TurnSnapshot) {
        let _ = self.input_tx.send((now, InEvent::NewLocalTurn(turn)));
    }

    /// 手动触发全量状态广播（等效于 NeighborUp 事件）。
    pub fn handle_neighbor_up(&self, now: Instant) {
        let _ = self.input_tx.send((now, InEvent::NeighborUp));
    }

    pub fn ticket(&self) -> &SyncTicket {
        &self.ticket
    }

    pub fn device_id(&self) -> DeviceId {
        self.device_id
    }

    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.secret_key
    }
}

// ── 内部 init ───────────────────────────────────────────────────────

async fn do_init(
    db: Arc<std::sync::Mutex<ChatDb>>,
    config: SyncConfig,
    secret_key: Option<[u8; 32]>,
    ticket: Option<SyncTicket>,
) -> SyncEngineResult<SyncEngine> {
    // 1. 初始化网络
    let secret_key = match secret_key {
        Some(bytes) => SecretKey::from_bytes(&bytes),
        None => SecretKey::generate(),
    };
    let secret_key_bytes = secret_key.to_bytes();

    // 2. 从私钥派生设备标识
    let device_id = DeviceId::from_secret_key(&secret_key_bytes);
    let signing_key = SigningKey::from_bytes(&secret_key_bytes);

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
    let sync_ticket = ticket.unwrap_or_default();
    let topic = init_topic(sync_ticket.clone().into(), gossip.clone(), signing_key).await?;

    info!("同步已启动");

    // 3. 构建水位 → 转入同步态
    let watermarks = {
        let guard = lock_db(&db)?;
        guard.build_watermarks(device_id)?
    };
    let machine = SyncMachine::new(device_id, config).into_syncing(sync_ticket.clone(), watermarks);

    // 4. 创建输入通道
    let (input_tx, input_rx) = mpsc::unbounded_channel::<(Instant, InEvent)>();

    // 5. 获取 gossip receiver
    let gossip_rx = topic.gossip_receiver().await.map_err(sync_net_err)?;

    // 6. 启动后台事件循环
    let bg = spawn_event_loop(
        machine,
        db.clone(),
        endpoint,
        topic.clone(),
        gossip_rx,
        input_rx,
    );

    Ok(SyncEngine {
        net,
        doc: DocRes { topic },
        db,
        input_tx,
        ticket: sync_ticket,
        device_id,
        secret_key: secret_key_bytes,
        _bg: bg,
    })
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

fn spawn_event_loop(
    mut machine: SyncMachine<SyncSyncing>,
    db: Arc<std::sync::Mutex<ChatDb>>,
    endpoint: Endpoint,
    topic: distributed_topic_tracker::Topic,
    mut gossip_rx: GossipReceiver,
    mut input_rx: mpsc::UnboundedReceiver<(Instant, InEvent)>,
) -> BackgroundSyncHandle {
    let (cancel_tx, mut cancel_rx) = tokio::sync::oneshot::channel();

    // 用于 P2P 回传的输入通道
    let (backfill_tx, mut backfill_rx) = mpsc::unbounded_channel::<(Instant, InEvent)>();

    tokio::spawn(async move {
        // ── 初始 NeighborUp 广播 ──────────────────────────────────
        let now = Instant::now();
        for out in machine.handle(now, InEvent::NeighborUp) {
            if let Err(e) = dispatch_out(&topic, &db, &endpoint, &backfill_tx, out).await {
                error!(%e, "初始广播失败");
            }
        }

        info!("同步事件循环已启动");

        loop {
            let timeout = machine.poll_timeout().and_then(|t| {
                let now = Instant::now();
                t.checked_duration_since(now)
            });

            tokio::select! {
                biased;

                // ── 取消信号 ────────────────────────────────────
                _ = &mut cancel_rx => {
                    info!("同步事件循环收到取消信号");
                    break;
                }

                // ── P2P 回传 ────────────────────────────────────
                Some((now, event)) = backfill_rx.recv() => {
                    for out in machine.handle(now, event) {
                        if let Err(e) = dispatch_out(&topic, &db, &endpoint, &backfill_tx, out).await {
                            error!(%e, "处理回传事件失败");
                        }
                    }
                }

                // ── 外部输入（handle_new_turn 等） ──────────────
                Some((now, event)) = input_rx.recv() => {
                    for out in machine.handle(now, event) {
                        if let Err(e) = dispatch_out(&topic, &db, &endpoint, &backfill_tx, out).await {
                            error!(%e, "处理输入事件失败");
                        }
                    }
                }

                // ── Gossip 消息 ──────────────────────────────────
                Ok(msg) = gossip_rx.next() => {
                    if let Err(e) = handle_gossip_event(
                        msg, &mut machine,
                        &topic, &db, &endpoint, &backfill_tx,
                    ).await {
                        error!(%e, "处理 gossip 事件失败");
                    }
                }

                // ── 定期超时 ─────────────────────────────────────
                _ = async {
                    if let Some(dur) = timeout {
                        tokio::time::sleep(dur).await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    let now = Instant::now();
                    for out in machine.handle(now, InEvent::Timeout) {
                        if let Err(e) = dispatch_out(&topic, &db, &endpoint, &backfill_tx, out).await {
                            error!(%e, "处理超时事件失败");
                        }
                    }
                }
            }
        }
    });

    BackgroundSyncHandle {
        cancel: Some(cancel_tx),
    }
}

// ── Gossip 事件处理 ─────────────────────────────────────────────────

async fn handle_gossip_event(
    event: GossipEvent,
    machine: &mut SyncMachine<SyncSyncing>,
    topic: &distributed_topic_tracker::Topic,
    db: &Arc<std::sync::Mutex<ChatDb>>,
    endpoint: &Endpoint,
    backfill_tx: &mpsc::UnboundedSender<(Instant, InEvent)>,
) -> SyncEngineResult<()> {
    match event {
        GossipEvent::Received(msg) => {
            match serde_json::from_slice::<GossipMessage>(&msg.content) {
                Ok(GossipMessage::TurnBroadcast(tb)) => {
                    let from_device = tb.device_id;
                    let event = InEvent::RemoteTurn {
                        from_device,
                        turn: tb.into_turn_snapshot(),
                    };
                    let now = Instant::now();
                    for out in machine.handle(now, event) {
                        dispatch_out(topic, db, endpoint, backfill_tx, out).await?;
                    }
                }
                Ok(GossipMessage::StateBroadcast(sb)) => {
                    let from_device = sb.device_id;
                    let event = InEvent::RemoteState {
                        from_device,
                        sessions: sb.sessions,
                    };
                    let now = Instant::now();
                    for out in machine.handle(now, event) {
                        dispatch_out(topic, db, endpoint, backfill_tx, out).await?;
                    }
                }
                Err(_) => {
                    // 可能是旧版 SyncAnnouncement 格式，尝试兼容解析
                    if let Ok(ann) =
                        serde_json::from_slice::<chat_pm_sync::SyncAnnouncement>(&msg.content)
                    {
                        let from_device = ann.device_id;
                        let event = InEvent::RemoteState {
                            from_device,
                            sessions: ann.sessions,
                        };
                        let now = Instant::now();
                        for out in machine.handle(now, event) {
                            dispatch_out(topic, db, endpoint, backfill_tx, out).await?;
                        }
                    } else {
                        warn!("解析 gossip 消息失败（非 GossipMessage 也非 SyncAnnouncement）");
                    }
                }
            }
        }
        GossipEvent::NeighborUp(peer) => {
            info!(%peer, "邻居上线——广播全量状态");
            let now = Instant::now();
            for out in machine.handle(now, InEvent::NeighborUp) {
                dispatch_out(topic, db, endpoint, backfill_tx, out).await?;
            }
        }
        GossipEvent::NeighborDown(peer) => info!(%peer, "邻居下线"),
        GossipEvent::Lagged => warn!("gossip 消息滞后"),
    }
    Ok(())
}

// ── OutEvent 分发 ──────────────────────────────────────────────────

/// 执行状态机产出的单个 [`OutEvent`]。
///
/// - `BroadcastGossip` → 通过 gossip 广播
/// - `WriteTurn` → 写入 DB
/// - `WriteSession` → 写入 DB
/// - `RequestBackfill` → 发起 P2P 请求，结果通过 `backfill_tx` 回传状态机
async fn dispatch_out(
    topic: &distributed_topic_tracker::Topic,
    db: &Arc<std::sync::Mutex<ChatDb>>,
    endpoint: &Endpoint,
    backfill_tx: &mpsc::UnboundedSender<(Instant, InEvent)>,
    out: OutEvent,
) -> SyncEngineResult<()> {
    match out {
        OutEvent::BroadcastGossip(msg) => {
            let json = serde_json::to_vec(&msg)?;
            let sender = topic.gossip_sender().await.map_err(sync_net_err)?;
            sender.broadcast(json).await.map_err(sync_net_err)?;

            match msg {
                GossipMessage::TurnBroadcast(tb) => {
                    info!(
                        session = %tb.session_id,
                        turn_num = tb.turn_num,
                        "轮次广播已发送"
                    );
                }
                GossipMessage::StateBroadcast(sb) => {
                    info!(
                        kind = ?sb.kind,
                        sessions = sb.sessions.len(),
                        "水位广播已发送"
                    );
                }
            }
        }

        OutEvent::WriteTurn(turn) => {
            let guard = lock_db(db)?;
            guard.upsert_turn(&turn)?;
            info!(
                session = %turn.session_id,
                turn_num = turn.turn_num,
                "远程轮次已写入本地"
            );
        }

        OutEvent::WriteSession(snapshot) => {
            let guard = lock_db(db)?;
            let record = SessionRecord {
                session_id: snapshot.session_id,
                created_at: snapshot.created_at,
                title: snapshot.title,
                user_persona: None,
            };
            guard.upsert_session(record)?;
            info!(
                session = %snapshot.session_id,
                "远程会话已创建"
            );
        }

        OutEvent::RequestBackfill { to_device, request } => {
            let peer_id = device_to_endpoint(to_device);
            let endpoint = endpoint.clone();
            let tx = backfill_tx.clone();

            info!(
                to_device = %to_device,
                need_sessions = request.need_sessions.len(),
                need_turns = request.need_turns.len(),
                "发起 P2P 补传请求"
            );

            tokio::spawn(async move {
                match request_sync(&endpoint, peer_id, &request).await {
                    Ok(payload) => match parse_sync_payload(payload) {
                        Ok(verified) => {
                            let (sessions, turns) = verified.into_inner();
                            let now = Instant::now();
                            let session_count = sessions.len();
                            let turn_count = turns.len();
                            for s in sessions {
                                let _ = tx.send((now, InEvent::RemoteSession(s)));
                            }
                            for t in turns {
                                let _ = tx.send((
                                    now,
                                    InEvent::RemoteTurn {
                                        from_device: to_device,
                                        turn: t,
                                    },
                                ));
                            }
                            info!(sessions = session_count, turns = turn_count, "P2P 补传完成");
                        }
                        Err(e) => error!(%e, "P2P 补传数据校验失败"),
                    },
                    Err(e) => error!(%e, "P2P 补传请求失败"),
                }
            });
        }
    }

    Ok(())
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

/// 锁定数据库，PoisonError 转 `DatabaseLock`。
fn lock_db(
    db: &Arc<std::sync::Mutex<ChatDb>>,
) -> SyncEngineResult<std::sync::MutexGuard<'_, ChatDb>> {
    db.lock().map_err(|_| SyncEngineError::DatabaseLock)
}

fn sync_net_err(e: impl Into<anyhow::Error>) -> SyncEngineError {
    SyncEngineError::Network(e.into())
}

// ── 传输层 ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMessage {
    request: Option<SyncRequest>,
    payload: Option<SyncPayload>,
}

impl SyncMessage {
    fn to_wire(&self) -> SyncEngineResult<Vec<u8>> {
        let json = serde_json::to_vec(self)?;
        let mut data = SYNC_PROTO_MAGIC.to_vec();
        data.extend_from_slice(&(json.len() as u32).to_be_bytes());
        data.extend_from_slice(&json);
        Ok(data)
    }

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

async fn handle_sync_connection(
    db: Arc<std::sync::Mutex<ChatDb>>,
    connection: iroh::endpoint::Connection,
) -> SyncEngineResult<()> {
    let (mut send, mut recv) = connection.accept_bi().await.map_err(sync_net_err)?;

    let buf = recv.read_to_end(1024 * 1024).await.map_err(sync_net_err)?;
    let msg = SyncMessage::from_wire(&buf)?;
    let request = msg.into_request()?;

    info!(
        need_sessions = request.need_sessions.len(),
        need_turns = request.need_turns.len(),
        "收到同步请求"
    );

    let payload = build_sync_payload(&db, &request)?;
    let sessions_count = payload.sessions.len();
    let turns_count = payload.turns.len();

    info!(
        sessions = sessions_count,
        turns = turns_count,
        "组装同步响应"
    );

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

fn build_sync_payload(
    db: &Arc<std::sync::Mutex<ChatDb>>,
    request: &SyncRequest,
) -> SyncEngineResult<SyncPayload> {
    let guard = lock_db(db)?;

    let mut sessions = Vec::new();
    let mut turns = Vec::new();

    for session_id in &request.need_sessions {
        if let Some(snapshot) = guard.get_session_snapshot(*session_id)? {
            sessions.push(snapshot);
            let session_turns = guard.get_turns_from(*session_id, 1)?;
            turns.extend(session_turns);
        }
    }

    for (session_id, start_turn) in &request.need_turns {
        let session_turns = guard.get_turns_from(*session_id, *start_turn)?;
        turns.extend(session_turns);
    }

    Ok(SyncPayload { sessions, turns })
}
