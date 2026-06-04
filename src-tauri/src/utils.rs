use crate::error::{AppError, ErrorKind};
use crate::payload::SyncStatusPayload;
use crate::payload::DEFAULT_MODEL;
use crate::state::AppState;
use chat_pm_database::ChatDb;
use chat_pm_service::session::{ChatConfig, ChatService};
use chat_pm_sync::DeviceId;
use std::str::FromStr;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tracing::{info, warn};

// ── 身份与密钥 ──────────────────────────────────────────────────────

pub(crate) fn load_or_create_identity(
    db: &Arc<std::sync::Mutex<ChatDb>>,
) -> Result<(DeviceId, [u8; 32]), AppError> {
    let guard = db.lock().map_err(|_| AppError::locked())?;
    if let Some(hex) = guard.get_config("device_secret_key")? {
        let secret_bytes = hex_to_bytes(&hex)
            .ok_or_else(|| AppError::new(ErrorKind::Internal, "无效的设备密钥"))?;
        let device_id = DeviceId::from_secret_key(&secret_bytes);
        Ok((device_id, secret_bytes))
    } else {
        let (device_id, key_bytes) = DeviceId::generate_identity();
        guard.set_config("device_secret_key", &bytes_to_hex(&key_bytes))?;
        Ok((device_id, key_bytes))
    }
}

pub(crate) fn build_service(db: &ChatDb, raw_key: &str) -> Result<ChatService, AppError> {
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

// ── Hex 编解码 ──────────────────────────────────────────────────────

pub(crate) fn bytes_to_hex(bytes: &[u8; 32]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub(crate) fn hex_to_bytes(hex: &str) -> Option<[u8; 32]> {
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

// ── 同步引擎恢复 ────────────────────────────────────────────────────

pub(crate) async fn restore_sync_engine(app: tauri::AppHandle, db: Arc<std::sync::Mutex<ChatDb>>) {
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

    let ticket = match chat_pm_service::sync_engine::SyncTicket::from_str(&ticket_str) {
        Ok(t) => t,
        Err(e) => {
            warn!("恢复同步引擎失败 (ticket parse): {e}");
            mark_sync_inactive(&db);
            return;
        }
    };

    let engine = match chat_pm_service::sync_engine::SyncEngine::join(
        db.clone(),
        chat_pm_service::sync_engine::SyncConfig::default(),
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
