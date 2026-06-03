use chat_pm_commands::session::{ChatConfig, ChatService, CommandError};
use chat_pm_commands::sync_engine::{SyncConfig, SyncEngine, SyncTicket};
use chat_pm_database::ChatDb;
use chat_pm_session::{
    message::UserInput,
    session::{NewSession, SessionId},
    ChatError,
};
use chat_pm_sync::{DeviceId, TurnSnapshot};
use serde::Serialize;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tauri::{async_runtime, Emitter, Manager, State};
use tokio::sync::Mutex;
use tracing::{info, warn};

mod error;
use error::{AppError, ErrorKind};

// ── State ───────────────────────────────────────────────────────────

struct AppState {
    db: Arc<std::sync::Mutex<ChatDb>>,
    db_path: std::path::PathBuf,
    service: Mutex<Option<ChatService>>,
    sync_engine: Mutex<Option<SyncEngine>>,
    device_id: DeviceId,
}

// ── Payload types for events & responses ────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct ChatChunkPayload {
    session_id: SessionId,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatDonePayload {
    session_id: SessionId,
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionTitlePayload {
    session_id: SessionId,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
struct SessionDeletedPayload {
    session_id: SessionId,
}

#[derive(Debug, Clone, Serialize)]
struct SessionInfo {
    session_id: SessionId,
    created_at: String,
    title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TurnInfo {
    turn_uuid: String,
    turn_num: u64,
    user_text: String,
    assistant_text: String,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
struct SyncStatusPayload {
    status: String,
    active: bool,
    ticket: Option<String>,
}

// ── Helper ──────────────────────────────────────────────────────────

/// Supported models for the UI to present.
const SUPPORTED_MODELS: &[&str] = &["deepseek-v4-flash", "deepseek-v4-pro"];
const DEFAULT_MODEL: &str = "deepseek-v4-flash";

/// 加载或生成设备身份密钥（ed25519），并派生 DeviceId。
///
/// 首次启动时生成随机密钥并持久化；后续启动从 DB 读取。
/// DeviceId = 公钥的 32 字节。
fn load_or_create_identity(
    db: &Arc<std::sync::Mutex<ChatDb>>,
) -> Result<(DeviceId, [u8; 32]), AppError> {
    let guard = db.lock().map_err(|_| AppError::locked())?;
    if let Some(hex) = guard.get_config("device_secret_key")? {
        let secret_bytes =
            hex_to_bytes(&hex).ok_or_else(|| AppError::new(ErrorKind::Internal, "无效的设备密钥"))?;
        let device_id = DeviceId::from_secret_key(&secret_bytes);
        Ok((device_id, secret_bytes))
    } else {
        let (device_id, key_bytes) = DeviceId::generate_identity();
        guard.set_config("device_secret_key", &bytes_to_hex(&key_bytes))?;
        Ok((device_id, key_bytes))
    }
}

/// Try to build a ChatService from a stored API key string, optionally with a stored model.
fn build_service(db: &ChatDb, raw_key: &str) -> Result<ChatService, AppError> {
    let key = chat_pm_deepseek::ApiKey::new(raw_key)
        .ok_or_else(|| AppError::new(ErrorKind::Validation, "Invalid API key"))?;
    let client = chat_pm_deepseek::Client::new(key);
    let model = db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string());
    let mut config = ChatConfig::default();
    config.set_chat_model(&model)?;
    ChatService::new(db.clone(), client, config).map_err(AppError::from)
}

fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_to_bytes(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut arr = [0u8; 32];
    for (i, chunk) in hex.as_bytes().chunks(2).enumerate() {
        let high = hex_char_to_val(chunk[0])?;
        let low = hex_char_to_val(chunk[1])?;
        arr[i] = (high << 4) | low;
    }
    Some(arr)
}

fn hex_char_to_val(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

async fn restore_sync_engine(
    app: tauri::AppHandle,
    db: Arc<std::sync::Mutex<ChatDb>>,
) {
    let (secret_key_bytes, ticket_str) = {
        let guard = match db.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let state = guard.get_config("sync_state").ok().flatten();
        if state.as_deref() != Some("active") {
            return;
        }
        let sk = match guard.get_config("sync_secret_key").ok().flatten() {
            Some(s) => s,
            None => {
                warn!("sync_state is active but sync_secret_key is missing");
                return;
            }
        };
        let ticket = match guard.get_config("sync_ticket").ok().flatten() {
            Some(t) => t,
            None => {
                warn!("sync_state is active but sync_ticket is missing");
                return;
            }
        };

        let sk_bytes = match hex_to_bytes(&sk) {
            Some(b) => b,
            None => {
                warn!("invalid sync_secret_key hex");
                return;
            }
        };

        (sk_bytes, ticket)
    };

    info!("正在恢复同步引擎...");

    let ticket = match SyncTicket::from_str(&ticket_str) {
        Ok(t) => t,
        Err(e) => {
            warn!("恢复同步引擎失败 (ticket parse): {e}");
            mark_sync_inactive(&db);
            return;
        }
    };

    let engine = match SyncEngine::join(
        db.clone(),
        SyncConfig::default(),
        Some(secret_key_bytes),
        ticket,
    )
    .await
    {
        Ok(e) => e,
        Err(e) => {
            warn!("恢复同步引擎失败: {e}");
            mark_sync_inactive(&db);
            return;
        }
    };

    let state = app.state::<AppState>();
    *state.sync_engine.lock().await = Some(engine);

    info!("同步引擎已自动恢复");

    let _ = app.emit(
        "sync-status-changed",
        SyncStatusPayload {
            status: "syncing".to_string(),
            active: true,
            ticket: None,
        },
    );
}

fn mark_sync_inactive(db: &Arc<std::sync::Mutex<ChatDb>>) {
    if let Ok(guard) = db.lock() {
        let _ = guard.set_config("sync_state", "inactive");
    }
}

// ── Commands ────────────────────────────────────────────────────────

#[tauri::command]
async fn check_api_key(state: State<'_, AppState>) -> Result<bool, AppError> {
    let guard = state.service.lock().await;
    Ok(guard.is_some())
}

#[tauri::command]
fn create_session(state: State<'_, AppState>) -> Result<String, AppError> {
    let guard = state.service.try_lock().map_err(|_| AppError::locked())?;
    let service = guard.as_ref().ok_or_else(AppError::not_configured)?;
    let new_session = service.create_session()?;
    Ok(new_session.session_id().to_string())
}

#[tauri::command]
fn set_api_key(state: State<'_, AppState>, api_key: String) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let service = build_service(&db, &api_key)?;
    db.set_config("api_key", &api_key)?;
    drop(db);

    let mut guard = state.service.try_lock().map_err(|_| AppError::locked())?;
    *guard = Some(service);

    tracing::info!("API key configured and persisted");
    Ok(())
}

#[tauri::command]
fn get_model(state: State<'_, AppState>) -> Result<String, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    Ok(db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string()))
}

#[tauri::command]
fn set_model(state: State<'_, AppState>, model: &str) -> Result<(), AppError> {
    let model = model.trim().to_ascii_lowercase();
    if !SUPPORTED_MODELS.contains(&model.as_str()) {
        return Err(AppError::new(
            ErrorKind::Validation,
            format!(
                "Unsupported model '{}', available: {}",
                model,
                SUPPORTED_MODELS.join(", ")
            ),
        ));
    }

    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    db.set_config("model", &model)?;

    // If service already exists, rebuild with new model
    if let Some(raw_key) = db.get_config("api_key").ok().flatten() {
        let service = build_service(&db, &raw_key)?;
        drop(db);
        let mut guard = state.service.try_lock().map_err(|_| AppError::locked())?;
        *guard = Some(service);
    } else {
        drop(db);
    }

    tracing::info!(%model, "Model switched");
    Ok(())
}

#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: &str,
    content: &str,
) -> Result<(), AppError> {
    let service = {
        let guard = state.service.lock().await;
        guard.clone().ok_or_else(AppError::not_configured)?
    };

    let session_id = session_id
        .parse::<SessionId>()
        .map_err(|e| AppError::new(ErrorKind::Validation, format!("Invalid session ID: {e}")))?;
    let user_input = UserInput::new(content);

    // ── 状态机：NewSession / Session 分流 ──────────────────────
    let session = match service.resume_session(session_id) {
        Ok(session) => session,
        Err(CommandError::Chat(
            ChatError::SessionNotFound(_) | ChatError::TitleNotGenerated(_),
        )) => {
            // 无标题或会话不存在 → 走 TitlePrompt → Session 流程
            let new_session = NewSession::with_id(session_id);
            let tp = new_session.into_title_prompt(&user_input);
            let session = service.finalize_session(tp).await?;

            let _ = app.emit(
                "session-title-updated",
                SessionTitlePayload {
                    session_id: session.session_id(),
                    title: session.title().to_string(),
                },
            );

            session
        }
        Err(e) => return Err(e.into()),
    };

    let mut stream = service.chat(&session, user_input).await?;

    let sid = session.session_id();
    let app_handle = app.clone();

    // Spawn a task to forward streaming chunks as Tauri events
    tokio::spawn(async move {
        let mut prompt_tokens = None;
        let mut completion_tokens = None;

        while let Some(result) = stream.recv().await {
            match result {
                Ok(frame) => {
                    if let Some(pt) = frame.prompt_tokens {
                        prompt_tokens = Some(pt);
                    }
                    if let Some(ct) = frame.completion_tokens {
                        completion_tokens = Some(ct);
                    }
                    let _ = app_handle.emit(
                        "chat-chunk",
                        ChatChunkPayload {
                            session_id: sid,
                            content: frame.content,
                        },
                    );
                }
                Err(e) => {
                    tracing::error!(%sid, "stream error: {e}");
                    break;
                }
            }
        }
        let _ = app_handle.emit(
            "chat-done",
            ChatDonePayload {
                session_id: sid,
                prompt_tokens,
                completion_tokens,
            },
        );

        // ── 通知同步引擎 ──────────────────────────────────────
        let app_state = app_handle.state::<AppState>();
        let turn_snapshot = {
            let db_guard = match app_state.db.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            match db_guard.recent_turns(sid, 1) {
                Ok(turns) => {
                    turns.first().map(|t| TurnSnapshot {
                        turn_id: t.turn_id,
                        session_id: t.session_id,
                        turn_num: t.turn_num,
                        user_text: t.user_text.clone(),
                        assistant_text: t.assistant_text.clone(),
                        created_at: t.created_at,
                        device_id: t.device_id.unwrap_or(app_state.device_id),
                    })
                }
                Err(_) => None,
            }
        };
        if let Some(snapshot) = turn_snapshot {
            let engine_guard = app_state.sync_engine.lock().await;
            if let Some(ref engine) = *engine_guard {
                engine.handle_new_turn(Instant::now(), snapshot);
            }
        }
    });

    Ok(())
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sessions = db.list_sessions()?;
    Ok(sessions
        .into_iter()
        .map(|s| SessionInfo {
            session_id: s.session_id,
            created_at: s.created_at.to_rfc3339(),
            title: s.title,
        })
        .collect())
}

#[tauri::command]
fn get_turns(state: State<'_, AppState>, session_id: &str) -> Result<Vec<TurnInfo>, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sid = session_id
        .parse::<SessionId>()
        .map_err(|e| AppError::new(ErrorKind::Validation, format!("Invalid session ID: {e}")))?;
    let turns = db.recent_turns(sid, 1000)?;
    let infos: Vec<TurnInfo> = turns
        .into_iter()
        .map(|t| TurnInfo {
            turn_uuid: t.turn_id.to_string(),
            turn_num: t.turn_num,
            user_text: t.user_text,
            assistant_text: t.assistant_text,
            prompt_tokens: t.prompt_tokens,
            completion_tokens: t.completion_tokens,
        })
        .collect();
    Ok(infos)
}

#[tauri::command]
fn update_session_title(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: &str,
    title: &str,
) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sid = session_id
        .parse::<SessionId>()
        .map_err(|e| AppError::new(ErrorKind::Validation, format!("Invalid session ID: {e}")))?;
    db.set_session_title(sid, title)?;
    drop(db);
    let _ = app.emit(
        "session-title-updated",
        SessionTitlePayload {
            session_id: sid,
            title: title.to_string(),
        },
    );
    Ok(())
}

#[tauri::command]
fn delete_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: &str,
) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sid = session_id
        .parse::<SessionId>()
        .map_err(|e| AppError::new(ErrorKind::Validation, format!("Invalid session ID: {e}")))?;
    db.delete_session(sid)?;
    drop(db);
    let _ = app.emit("session-deleted", SessionDeletedPayload { session_id: sid });
    Ok(())
}

#[tauri::command]
fn clear_all_data(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    // 1. Drop service
    *state.service.try_lock().map_err(|_| AppError::locked())? = None;

    // 2. Drop old db connection
    let placeholder = ChatDb::open_in_memory()?;
    let old_db = {
        let mut guard = state.db.try_lock().map_err(|_| AppError::locked())?;
        std::mem::replace(&mut *guard, placeholder)
    };
    drop(old_db);

    // 3. Delete database files (ignore not-found errors)
    let _ = std::fs::remove_file(&state.db_path);
    let _ = std::fs::remove_file(state.db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(state.db_path.with_extension("db-shm"));

    // 4. Create fresh database
    let new_db = ChatDb::open(&state.db_path)?;
    *state.db.try_lock().map_err(|_| AppError::locked())? = new_db;

    // 5. Emit event
    app.emit("data-cleared", ())
        .map_err(|e| AppError::new(ErrorKind::Internal, e.to_string()))?;

    tracing::info!("All data cleared, database rebuilt");
    Ok(())
}

// ── Sync commands ──────────────────────────────────────────────────

#[tauri::command]
async fn get_sync_status(state: State<'_, AppState>) -> Result<SyncStatusPayload, AppError> {
    let guard = state.sync_engine.lock().await;
    let active = guard.is_some();
    let ticket = if active {
        let db = state.db.lock().map_err(|_| AppError::locked())?;
        db.get_config("sync_ticket").ok().flatten()
    } else {
        None
    };
    Ok(SyncStatusPayload {
        status: if active {
            "syncing".to_string()
        } else {
            "inactive".to_string()
        },
        active,
        ticket,
    })
}

#[tauri::command]
async fn init_and_create_sync_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let db = Arc::clone(&state.db);

    let engine =
        SyncEngine::create(db.clone(), SyncConfig::default(), None).await?;
    let ticket = engine.ticket().to_string();
    let secret_key_bytes = engine.secret_key_bytes();

    {
        let guard = db.lock().map_err(|_| AppError::locked())?;
        guard.set_config("sync_state", "active")?;
        guard.set_config("sync_role", "creator")?;
        guard.set_config("sync_ticket", &ticket)?;
        guard.set_config("sync_secret_key", &bytes_to_hex(&secret_key_bytes))?;
    }

    *state.sync_engine.lock().await = Some(engine);

    let _ = app.emit("sync-status-changed", SyncStatusPayload {
        status: "syncing".to_string(), active: true, ticket: None,
    });

    Ok(ticket)
}

#[tauri::command]
async fn join_sync_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ticket: String,
) -> Result<(), AppError> {
    let db = Arc::clone(&state.db);
    let doc_ticket = SyncTicket::from_str(&ticket)?;

    let engine =
        SyncEngine::join(db.clone(), SyncConfig::default(), None, doc_ticket).await?;
    let secret_key_bytes = engine.secret_key_bytes();

    {
        let guard = db.lock().map_err(|_| AppError::locked())?;
        guard.set_config("sync_state", "active")?;
        guard.set_config("sync_role", "joiner")?;
        guard.set_config("sync_ticket", &ticket)?;
        guard.set_config("sync_secret_key", &bytes_to_hex(&secret_key_bytes))?;
    }

    *state.sync_engine.lock().await = Some(engine);

    let _ = app.emit("sync-status-changed", SyncStatusPayload {
        status: "syncing".to_string(), active: true, ticket: None,
    });

    Ok(())
}

#[tauri::command]
async fn stop_sync(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    let mut guard = state.sync_engine.lock().await;
    let _old_engine = guard.take();

    // Mark sync as inactive in database
    {
        let db = state.db.lock().map_err(|_| AppError::locked())?;
        db.set_config("sync_state", "inactive")?;
    }

    let _ = app.emit(
        "sync-status-changed",
        SyncStatusPayload {
            status: "inactive".to_string(),
            active: false,
            ticket: None,
        },
    );

    Ok(())
}

#[tauri::command]
async fn publish_sync_announcement(state: State<'_, AppState>) -> Result<(), AppError> {
    let guard = state.sync_engine.lock().await;
    let engine = guard
        .as_ref()
        .ok_or_else(|| AppError::new(ErrorKind::Validation, "同步引擎未启动"))?;
    engine.handle_neighbor_up(Instant::now());
    Ok(())
}

// ── Entry point ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .targets([tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                )])
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("chatpm.db");

            let raw_db = ChatDb::open(&db_path)?;
            tracing::info!(path = %db_path.display(), "Database opened");

            let db = Arc::new(std::sync::Mutex::new(raw_db));

            let (device_id, _identity_key) = load_or_create_identity(&db)?;

            let service = {
                let guard = db.lock().map_err(|_| AppError::locked())?;
                guard
                    .get_config("api_key")
                    .ok()
                    .flatten()
                    .and_then(|raw_key| build_service(&guard, &raw_key).ok())
            };

            if service.is_some() {
                tracing::info!("API key restored from database");
            }

            app.manage(AppState {
                db: Arc::clone(&db),
                db_path,
                service: Mutex::new(service),
                sync_engine: Mutex::new(None),
                device_id,
            });

            // Auto-restore sync engine if it was active before shutdown
            let handle = app.handle().clone();
            let restore_db = Arc::clone(&db);
            async_runtime::spawn(async move {
                restore_sync_engine(handle, restore_db).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_api_key,
            create_session,
            set_api_key,
            get_model,
            set_model,
            send_message,
            list_sessions,
            get_turns,
            update_session_title,
            delete_session,
            clear_all_data,
            get_sync_status,
            init_and_create_sync_doc,
            join_sync_doc,
            stop_sync,
            publish_sync_announcement,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}
