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
        .alpns(vec![SYNC_ALPN.to_vec()])
        .address_lookup(mdns)
        .bind()
        .await
        .map_err(sync_net_err)?;

    let gossip = Gossip::builder().spawn(endpoint.clone());
    let _router = iroh::protocol::Router::builder(endpoint.clone())
        .accept(iroh_gossip::ALPN, gossip.clone())
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
    let topic =
        init_docs_and_topic(sync_ticket.clone().into(), gossip.clone(), signing_key).await?;

    info!("同步已启动");

    // 4. 构建水位 → 转入同步态（拿到需广播的 Announcement）
    let watermarks = {
        let guard = db
            .lock()
            .map_err(|_| SyncEngineError::Internal(anyhow::anyhow!("数据库锁已污染")))?;
        guard.build_watermarks(device_id)?
    };
    let (machine, announcement) =
        SyncMachine::new(device_id, config).into_syncing(sync_ticket, watermarks);

    // 5. 广播
    let json = serde_json::to_vec(&announcement)?;
    let sender = topic.gossip_sender().await.map_err(sync_net_err)?;
    sender.broadcast(json).await.map_err(sync_net_err)?;
    info!(sessions = announcement.sessions.len(), "初始广播已发送");

    // 6. 事件通道
    let (event_tx, _) = broadcast::channel::<SyncEvent>(16);

    let topic_for_bg = topic.clone();

    // 7. 启动后台监听
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
        let guard = db
            .lock()
            .map_err(|_| SyncEngineError::Internal(anyhow::anyhow!("数据库锁已污染")))?;
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

async fn init_docs_and_topic(
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
                .max_bootstrap_records(5)
                .publish_record_on_startup(true)
                .check_older_records_first_on_startup(false)
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

    info!("gossip topic 已加入");
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
                    Ok(remote_announcement) => {
                        if remote_announcement.device_id == device_id {
                            continue;
                        }
                        info!(from_device = %remote_announcement.device_id, sessions = remote_announcement.sessions.len(), "收到远程同步公告");
                        let local_watermarks = {
                            let db_guard = match db.lock() {
                                Ok(g) => g,
                                Err(e) => {
                                    error!("数据库锁已污染: {e}");
                                    continue;
                                }
                            };
                            match db_guard.build_watermarks(device_id) {
                                Ok(w) => w,
                                Err(e) => {
                                    error!(%e, "构建水位失败");
                                    continue;
                                }
                            }
                        };
                        let request = compute_sync_request(&local_watermarks, &remote_announcement);
                        if request.need_sessions.is_empty() && request.need_turns.is_empty() {
                            continue;
                        }
                        info!(
                            need_sessions = request.need_sessions.len(),
                            need_turns = request.need_turns.len(),
                            "需要同步数据"
                        );
                        match request_sync(&endpoint, msg.delivered_from, &request).await {
                            Ok(payload) => match parse_sync_payload(payload) {
                                Ok(verified) => {
                                    let db_guard = match db.lock() {
                                        Ok(g) => g,
                                        Err(e) => {
                                            error!("数据库锁已污染: {e}");
                                            continue;
                                        }
                                    };
                                    match db_guard.apply_verified_payload(&verified) {
                                        Ok(count) => {
                                            info!(turns_written = count, "同步数据已写入本地数据库")
                                        }
                                        Err(e) => error!(%e, "写入同步数据失败"),
                                    }
                                }
                                Err(e) => error!(%e, "验证同步负载失败"),
                            },
                            Err(e) => error!(%e, "同步请求失败"),
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

// ── 传输层 ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMessage {
    request: Option<SyncRequest>,
    payload: Option<SyncPayload>,
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
    let mut send_data = SYNC_PROTO_MAGIC.to_vec();
    let json = serde_json::to_vec(&msg)?;
    send_data.extend_from_slice(&(json.len() as u32).to_be_bytes());
    send_data.extend_from_slice(&json);

    let (mut send, mut recv) = conn.open_bi().await.map_err(sync_net_err)?;
    send.write_all(&send_data).await.map_err(sync_net_err)?;
    send.finish()
        .map_err(|e| SyncEngineError::Network(e.into()))?;

    let buf = recv.read_to_end(1024 * 1024).await.map_err(sync_net_err)?;
    if buf.len() < 8 || &buf[..4] != SYNC_PROTO_MAGIC {
        return Err(SyncEngineError::Internal(anyhow::anyhow!(
            "无效的同步响应格式"
        )));
    }
    let json_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if buf.len() < 8 + json_len {
        return Err(SyncEngineError::Internal(anyhow::anyhow!(
            "同步响应数据不完整"
        )));
    }
    let response: SyncMessage = serde_json::from_slice(&buf[8..8 + json_len])?;
    response
        .payload
        .ok_or_else(|| SyncEngineError::Internal(anyhow::anyhow!("响应中无负载数据")))
}

fn sync_net_err(e: impl Into<anyhow::Error>) -> SyncEngineError {
    SyncEngineError::Network(e.into())
}
