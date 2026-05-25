//! 同步引擎——基于 iroh 的 P2P 设备间会话同步。
//!
//! `SyncEngine<S>` 是同步功能的核心编排器，通过类型状态机驱动完整生命周期。
//! 每个状态类型携带自身所需的具体数据，消除所有 `Option` 和运行时检查。
//! 所有底层 iroh 细节封装在内部，公共 API 不暴露任何 `iroh::` 类型。

use std::sync::Arc;

use chat_pm_database::ChatDb;
use chat_pm_sync::{
    DeviceId, DocTicket, SyncAnnouncement, SyncError, SyncPayload, SyncRequest,
    compute_sync_request, parse_sync_payload,
};
use distributed_topic_tracker::{AutoDiscoveryGossip, RecordPublisher, TopicId};
use ed25519_dalek::SigningKey;
use iroh::{Endpoint, EndpointId, SecretKey, endpoint::presets};
use iroh_docs::{
    DocTicket as IrohDocTicket,
    api::{
        Doc,
        protocol::{AddrInfoOptions, ShareMode},
    },
    protocol::Docs,
};
use iroh_gossip::{api::Event as GossipEvent, net::Gossip};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

// ── SyncConfig ──────────────────────────────────────────────────────

/// 同步引擎配置。
#[derive(Debug, Clone, Default)]
pub struct SyncConfig {
    /// iroh 端点绑定的端口（None = 系统自动分配）
    pub bind_port: Option<u16>,
    /// 设备名称（可选，用于 UI 区分）
    pub device_name: Option<String>,
}

// ── Topic 常量 ──────────────────────────────────────────────────────

/// 同步 topic 名称（用于 distributed-topic-tracker）
const SYNC_TOPIC_NAME: &str = "chatpm-sync-v1";

/// 直连同步请求的 ALPN
const SYNC_ALPN: &[u8] = b"/chatpm/sync-req/1";

/// 同步传输协议版本 magic（前 4 字节）
const SYNC_PROTO_MAGIC: &[u8; 4] = b"cSPM";

// ── State types (each carries its own concrete data) ────────────────

/// 未连接网络——不持有任何网络资源。
pub struct Disconnected;

/// 已加入 gossip 网络——持有端点、gossip 实例和签名密钥。
/// 此时尚未创建或加入同步文档。
pub struct Connected {
    endpoint: Endpoint,
    gossip: Gossip,
    signing_key: SigningKey,
}

/// Authoring / Joined / Syncing 共享的内部数据——三者均持有完整的
/// 网络连接 + 文档 + gossip topic，差异仅在于允许的操作不同。
struct SyncedInner {
    endpoint: Endpoint,
    gossip: Gossip,
    signing_key: SigningKey,
    #[allow(dead_code)]
    docs: Docs,
    doc: Doc,
    topic: distributed_topic_tracker::Topic,
}

/// 已创建同步文档，持有 ticket——等待调用 `start()` 启动同步。
pub struct Authoring {
    inner: SyncedInner,
}

/// 已凭 ticket 加入同步文档——等待调用 `start()` 启动同步。
pub struct Joined {
    inner: SyncedInner,
}

/// 正在同步中——可以发布公告、接收远程数据。
pub struct Syncing {
    inner: SyncedInner,
}

// ── SyncEngine ──────────────────────────────────────────────────────

/// 同步引擎，`S` 为当前生命周期状态（同时携带该状态的具体数据）。
///
/// 状态转移：
/// ```text
/// Disconnected → init() → Connected → create_doc() → Authoring → start() → Syncing
///                                     → join_doc()  → Joined   → start() → Syncing
/// Syncing → stop() → Connected
/// ```
pub struct SyncEngine<S> {
    state: S,
    db: Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    config: SyncConfig,
}

// ── Disconnected ────────────────────────────────────────────────────

impl SyncEngine<Disconnected> {
    /// 初始化网络连接：创建端点、加入 gossip 网络。
    ///
    /// 可通过 `secret_key` 恢复之前的节点身份（用于重启后恢复同步）。
    pub async fn init(
        db: Arc<std::sync::Mutex<ChatDb>>,
        config: SyncConfig,
        device_id: DeviceId,
        secret_key: Option<[u8; 32]>,
    ) -> Result<SyncEngine<Connected>, SyncError> {
        let secret_key = match secret_key {
            Some(bytes) => SecretKey::from_bytes(&bytes),
            None => SecretKey::generate(),
        };
        let secret_key_bytes = secret_key.to_bytes();
        let signing_key = SigningKey::from_bytes(&secret_key_bytes);

        let endpoint = Endpoint::builder(presets::N0)
            .secret_key(secret_key)
            .alpns(vec![SYNC_ALPN.to_vec()])
            .bind()
            .await
            .map_err(sync_net_err)?;

        let gossip = Gossip::builder().spawn(endpoint.clone());

        let _router = iroh::protocol::Router::builder(endpoint.clone())
            .accept(iroh_gossip::ALPN, gossip.clone())
            .spawn();

        info!(
            node_id = %endpoint.id(),
            "同步引擎网络已初始化"
        );

        Ok(SyncEngine {
            state: Connected {
                endpoint,
                gossip,
                signing_key,
            },
            db,
            device_id,
            config,
        })
    }
}

// ── secret_key_bytes helper macro ───────────────────────────────────

macro_rules! impl_secret_key_bytes {
    ($($state:ident),+ $(,)?) => {
        $(
            impl SyncEngine<$state> {
                /// 返回 `secret_key` 的字节表示（用于重启后恢复节点身份）。
                pub fn secret_key_bytes(&self) -> [u8; 32] {
                    self.state.signing_key.to_bytes()
                }
            }
        )+
    };
}

impl_secret_key_bytes!(Connected);

impl SyncEngine<Authoring> {
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.state.inner.signing_key.to_bytes()
    }
}

impl SyncEngine<Joined> {
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.state.inner.signing_key.to_bytes()
    }
}

impl SyncEngine<Syncing> {
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.state.inner.signing_key.to_bytes()
    }
}

// ── Connected ───────────────────────────────────────────────────────

impl SyncEngine<Connected> {
    /// 发起者：创建同步文档，获得 DocTicket。
    pub async fn create_doc(self) -> Result<(SyncEngine<Authoring>, DocTicket), SyncError> {
        let (docs, topic) = init_docs_and_topic(
            self.state.endpoint.clone(),
            self.state.gossip.clone(),
            self.state.signing_key.clone(),
        )
        .await?;

        let doc = docs.create().await.map_err(sync_net_err)?;

        let iroh_ticket = doc
            .share(ShareMode::Write, AddrInfoOptions::Id)
            .await
            .map_err(sync_net_err)?;

        let ticket_str = iroh_ticket.to_string();

        info!(ticket = %ticket_str, "同步文档已创建");

        Ok((
            SyncEngine {
                state: Authoring {
                    inner: SyncedInner {
                        endpoint: self.state.endpoint,
                        gossip: self.state.gossip,
                        signing_key: self.state.signing_key,
                        docs,
                        doc,
                        topic,
                    },
                },
                db: self.db,
                device_id: self.device_id,
                config: self.config,
            },
            DocTicket::from_string(ticket_str),
        ))
    }

    /// 加入者：凭 ticket 加入已有同步链。
    pub async fn join_doc(self, ticket: DocTicket) -> Result<SyncEngine<Joined>, SyncError> {
        let iroh_ticket: IrohDocTicket = ticket
            .as_str()
            .parse()
            .map_err(|e| SyncError::Other(format!("无效的同步凭证: {e}")))?;

        let (docs, topic) = init_docs_and_topic(
            self.state.endpoint.clone(),
            self.state.gossip.clone(),
            self.state.signing_key.clone(),
        )
        .await?;

        let (doc, _events) = docs
            .import_and_subscribe(iroh_ticket)
            .await
            .map_err(sync_net_err)?;

        info!(doc_id = %doc.id(), "已加入同步文档");

        Ok(SyncEngine {
            state: Joined {
                inner: SyncedInner {
                    endpoint: self.state.endpoint,
                    gossip: self.state.gossip,
                    signing_key: self.state.signing_key,
                    docs,
                    doc,
                    topic,
                },
            },
            db: self.db,
            device_id: self.device_id,
            config: self.config,
        })
    }
}

// ── Authoring ───────────────────────────────────────────────────────

impl SyncEngine<Authoring> {
    /// 发起者启动同步。
    pub async fn start(self) -> Result<SyncEngine<Syncing>, SyncError> {
        // doc 由类型保证存在——无需运行时检查
        self.state
            .inner
            .doc
            .start_sync(vec![])
            .await
            .map_err(sync_net_err)?;

        info!("同步已启动（发起者）");

        Ok(SyncEngine {
            state: Syncing {
                inner: self.state.inner,
            },
            db: self.db,
            device_id: self.device_id,
            config: self.config,
        })
    }
}

// ── Joined ──────────────────────────────────────────────────────────

impl SyncEngine<Joined> {
    /// 加入者启动同步。
    pub async fn start(self) -> Result<SyncEngine<Syncing>, SyncError> {
        info!("同步已启动（加入者）");

        Ok(SyncEngine {
            state: Syncing {
                inner: self.state.inner,
            },
            db: self.db,
            device_id: self.device_id,
            config: self.config,
        })
    }
}

// ── Syncing ─────────────────────────────────────────────────────────

impl SyncEngine<Syncing> {
    /// 本地发生变更后，发布状态广播。
    pub async fn publish_announcement(&self) -> Result<(), SyncError> {
        let watermarks = {
            let db = self
                .db
                .lock()
                .map_err(|_| SyncError::Other("数据库锁已污染".into()))?;
            db.build_watermarks(self.device_id)
                .map_err(|e| SyncError::Other(e.to_string()))?
        };

        let announcement = SyncAnnouncement {
            device_id: self.device_id,
            sessions: watermarks,
        };

        let json = serde_json::to_vec(&announcement)
            .map_err(|e| SyncError::Other(format!("序列化公告失败: {e}")))?;

        // topic 由类型保证存在——无需运行时检查
        let sender = self
            .state
            .inner
            .topic
            .gossip_sender()
            .await
            .map_err(sync_net_err)?;

        sender.broadcast(json).await.map_err(sync_net_err)?;

        info!(
            device_id = %self.device_id,
            sessions = announcement.sessions.len(),
            "同步公告已发布"
        );

        Ok(())
    }

    /// 开启后台同步循环（接收端逻辑）。
    #[must_use]
    pub fn start_background_sync(&self) -> BackgroundSyncHandle {
        let db = Arc::clone(&self.db);
        let device_id = self.device_id;
        let endpoint = self.state.inner.endpoint.clone();
        // topic 和 doc 由类型保证存在
        let topic = self.state.inner.topic.clone();
        let doc = self.state.inner.doc.clone();

        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            tokio::select! {
                _ = cancel_rx => {
                    info!("后台同步已取消");
                }
                _ = run_sync_loop(db, device_id, endpoint, topic, doc) => {
                    warn!("后台同步循环已退出");
                }
            }
        });

        BackgroundSyncHandle {
            cancel: Some(cancel_tx),
        }
    }

    /// 停止同步，回到已连接状态。
    pub async fn stop(self) -> Result<SyncEngine<Connected>, SyncError> {
        info!("同步已停止");

        Ok(SyncEngine {
            state: Connected {
                endpoint: self.state.inner.endpoint,
                gossip: self.state.inner.gossip,
                signing_key: self.state.inner.signing_key,
            },
            db: self.db,
            device_id: self.device_id,
            config: self.config,
        })
    }
}

// ── BackgroundSyncHandle ────────────────────────────────────────────

/// 后台同步循环的取消令牌。
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

// ── Internal helpers ────────────────────────────────────────────────

async fn init_docs_and_topic(
    endpoint: Endpoint,
    gossip: Gossip,
    signing_key: SigningKey,
) -> Result<(Docs, distributed_topic_tracker::Topic), SyncError> {
    let blobs = iroh_blobs::store::mem::MemStore::default();

    let docs = Docs::memory()
        .spawn(endpoint.clone(), blobs.into(), gossip.clone())
        .await
        .map_err(sync_net_err)?;

    let topic_id = TopicId::new(SYNC_TOPIC_NAME.to_string());
    let initial_secret = b"chatpm-sync-initial".to_vec();
    let record_publisher = RecordPublisher::new(
        topic_id,
        signing_key,
        None,
        initial_secret,
        distributed_topic_tracker::Config::default(),
    );

    let topic = gossip
        .subscribe_and_join_with_auto_discovery_no_wait(record_publisher)
        .await
        .map_err(sync_net_err)?;

    info!("gossip topic 已加入");

    Ok((docs, topic))
}

/// 后台同步循环：监听 gossip 事件，接收远程公告并拉取数据。
///
/// `topic` 和 `doc` 由调用方保证存在（类型系统已消除 Option）。
async fn run_sync_loop(
    db: Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
    endpoint: Endpoint,
    topic: distributed_topic_tracker::Topic,
    _doc: Doc,
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

                        info!(
                            from_device = %remote_announcement.device_id,
                            sessions = remote_announcement.sessions.len(),
                            "收到远程同步公告"
                        );

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

                        let peer_id = msg.delivered_from;

                        match request_sync(&endpoint, peer_id, &request).await {
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
                                            info!(
                                                turns_written = count,
                                                "同步数据已写入本地数据库"
                                            );
                                        }
                                        Err(e) => {
                                            error!(%e, "写入同步数据失败");
                                        }
                                    }
                                }
                                Err(e) => {
                                    error!(%e, "验证同步负载失败");
                                }
                            },
                            Err(e) => {
                                error!(%e, "同步请求失败");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(%e, "解析远程公告失败");
                    }
                }
            }
            Ok(GossipEvent::NeighborUp(peer)) => {
                info!(%peer, "邻居上线");
            }
            Ok(GossipEvent::NeighborDown(peer)) => {
                info!(%peer, "邻居下线");
            }
            Ok(GossipEvent::Lagged) => {
                warn!("gossip 消息滞后");
            }
            Err(e) => {
                error!(%e, "gossip receiver 错误");
                break;
            }
        }
    }
}

// ── Transport layer ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SyncMessage {
    request: Option<SyncRequest>,
    payload: Option<SyncPayload>,
}

/// 向对端发送同步请求并等待响应。
async fn request_sync(
    endpoint: &Endpoint,
    peer: EndpointId,
    request: &SyncRequest,
) -> Result<SyncPayload, SyncError> {
    let conn = endpoint
        .connect(peer, SYNC_ALPN)
        .await
        .map_err(sync_net_err)?;

    let msg = SyncMessage {
        request: Some(request.clone()),
        payload: None,
    };
    let mut send_data = SYNC_PROTO_MAGIC.to_vec();
    let json =
        serde_json::to_vec(&msg).map_err(|e| SyncError::Other(format!("序列化请求失败: {e}")))?;
    send_data.extend_from_slice(&(json.len() as u32).to_be_bytes());
    send_data.extend_from_slice(&json);

    let (mut send, mut recv) = conn.open_bi().await.map_err(sync_net_err)?;

    send.write_all(&send_data).await.map_err(sync_net_err)?;
    send.finish()
        .map_err(|e| SyncError::Other(format!("finish 失败: {e}")))?;

    let buf = recv.read_to_end(1024 * 1024).await.map_err(sync_net_err)?;

    if buf.len() < 8 || &buf[..4] != SYNC_PROTO_MAGIC {
        return Err(SyncError::Other("无效的同步响应格式".into()));
    }

    let json_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if buf.len() < 8 + json_len {
        return Err(SyncError::Other("同步响应数据不完整".into()));
    }

    let response: SyncMessage = serde_json::from_slice(&buf[8..8 + json_len])
        .map_err(|e| SyncError::Other(format!("解析响应失败: {e}")))?;

    response
        .payload
        .ok_or_else(|| SyncError::Other("响应中无负载数据".into()))
}

/// 处理接收到的同步请求。
async fn handle_sync_request(
    db: &Arc<std::sync::Mutex<ChatDb>>,
    _device_id: DeviceId,
    request: &SyncRequest,
) -> Result<SyncPayload, SyncError> {
    let db = db
        .lock()
        .map_err(|_| SyncError::Other("数据库锁已污染".into()))?;

    let mut sessions = Vec::new();
    let mut turns = Vec::new();

    for sid in &request.need_sessions {
        if let Some(snapshot) = db
            .get_session_snapshot(*sid)
            .map_err(|e| SyncError::Other(e.to_string()))?
        {
            sessions.push(snapshot);
            let all_turns = db
                .get_turns_from(*sid, 0)
                .map_err(|e| SyncError::Other(e.to_string()))?;
            turns.extend(all_turns);
        }
    }

    for (sid, start_turn) in &request.need_turns {
        let partial_turns = db
            .get_turns_from(*sid, *start_turn)
            .map_err(|e| SyncError::Other(e.to_string()))?;
        turns.extend(partial_turns);
    }

    Ok(SyncPayload { sessions, turns })
}

/// 启动同步请求处理服务（在端点上监听入站连接）。
#[allow(dead_code)]
async fn start_sync_server(
    endpoint: &Endpoint,
    db: Arc<std::sync::Mutex<ChatDb>>,
    device_id: DeviceId,
) -> Result<(), SyncError> {
    loop {
        let Some(incoming) = endpoint.accept().await else {
            continue;
        };

        // iroh 的 accept 返回 Connecting，直接 await 得到 Connection
        let Ok(conn) = incoming.await else {
            continue;
        };

        let db = Arc::clone(&db);
        tokio::spawn(async move {
            while let Ok((mut send, mut recv)) = conn.accept_bi().await {
                let buf = match recv.read_to_end(1024 * 1024).await {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                if buf.len() < 8 || &buf[..4] != SYNC_PROTO_MAGIC {
                    continue;
                }

                let json_len = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
                if buf.len() < 8 + json_len {
                    continue;
                }

                let msg: SyncMessage = match serde_json::from_slice(&buf[8..8 + json_len]) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                let Some(ref request) = msg.request else {
                    continue;
                };

                let response = match handle_sync_request(&db, device_id, request).await {
                    Ok(payload) => SyncMessage {
                        request: None,
                        payload: Some(payload),
                    },
                    Err(e) => {
                        error!(%e, "处理同步请求失败");
                        continue;
                    }
                };

                let mut resp_data = SYNC_PROTO_MAGIC.to_vec();
                let json = match serde_json::to_vec(&response) {
                    Ok(j) => j,
                    Err(_) => continue,
                };
                resp_data.extend_from_slice(&(json.len() as u32).to_be_bytes());
                resp_data.extend_from_slice(&json);

                let _ = send.write_all(&resp_data).await;
                let _ = send.finish();
            }
        });
    }
}

// ── Error helpers ───────────────────────────────────────────────────

fn sync_net_err(e: impl std::fmt::Display) -> SyncError {
    SyncError::Other(format!("网络错误: {e}"))
}
