use chat_pm_commands::session::{ChatConfig, ChatService, CommandError};
use chat_pm_database::MemoryDb;
use chat_pm_session::{
    message::UserInput,
    session::{NewSession, SessionId},
    ChatError,
};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

mod error;
use error::{AppError, ErrorKind};

// ── State ───────────────────────────────────────────────────────────

struct AppState {
    db: std::sync::Mutex<MemoryDb>,
    db_path: std::path::PathBuf,
    service: Mutex<Option<ChatService>>,
}

// ── Payload types for events & responses ────────────────────────────

#[derive(Debug, Clone, Serialize)]
struct ChatChunkPayload {
    session_id: String,
    content: String,
}

#[derive(Debug, Clone, Serialize)]
struct ChatDonePayload {
    session_id: String,
    prompt_tokens: Option<usize>,
    completion_tokens: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
struct SessionTitlePayload {
    session_id: String,
    title: String,
}

#[derive(Debug, Clone, Serialize)]
struct SessionDeletedPayload {
    session_id: String,
}

#[derive(Debug, Clone, Serialize)]
struct SessionInfo {
    session_id: String,
    created_at: String,
    title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct TurnInfo {
    turn_num: u64,
    user_text: String,
    assistant_text: String,
    prompt_tokens: Option<i64>,
    completion_tokens: Option<i64>,
}

// ── Helper ──────────────────────────────────────────────────────────

/// Try to build a ChatService from a stored API key string.
fn build_service(db: &MemoryDb, raw_key: &str) -> Result<ChatService, AppError> {
    let key = chat_pm_deepseek::ApiKey::new(raw_key)
        .ok_or_else(|| AppError::new(ErrorKind::Validation, "无效的 API Key"))?;
    let client = chat_pm_deepseek::Client::new(key);
    let config = ChatConfig::default();
    ChatService::new(db.clone(), client, config).map_err(AppError::from)
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
    let new_session = service.create_session().map_err(AppError::from)?;
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

    tracing::info!("API key 已配置并持久化");
    Ok(())
}

#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    content: String,
) -> Result<(), AppError> {
    let service = {
        let guard = state.service.lock().await;
        guard.clone().ok_or_else(AppError::not_configured)?
    };

    let session_id = SessionId::from_uuid(
        uuid::Uuid::parse_str(&session_id)
            .map_err(|e| AppError::new(ErrorKind::Validation, format!("无效的会话 ID: {e}")))?,
    );
    let user_input = UserInput::new(&content);

    // ── 状态机：NewSession / Session 分流 ──────────────────────
    let session = match service.resume_session(session_id) {
        Ok(session) => session,
        Err(CommandError::Chat(
            ChatError::SessionNotFound(_) | ChatError::TitleNotGenerated(_),
        )) => {
            // 无标题或会话不存在 → 走 TitlePrompt → Session 流程
            let new_session = NewSession::with_id(session_id);
            let tp = new_session.into_title_prompt(user_input.clone());
            let session = service.finalize_session(tp).await?;

            let _ = app.emit(
                "session-title-updated",
                SessionTitlePayload {
                    session_id: session.session_id().to_string(),
                    title: session.title().to_string(),
                },
            );

            session
        }
        Err(e) => return Err(e.into()),
    };

    let mut stream = service
        .chat(&session, user_input)
        .await
        .map_err(AppError::from)?;

    let sid_str = session.session_id().to_string();
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
                            session_id: sid_str.clone(),
                            content: frame.content,
                        },
                    );
                }
                Err(e) => {
                    tracing::error!(%sid_str, "stream error: {e}");
                    break;
                }
            }
        }
        let _ = app_handle.emit(
            "chat-done",
            ChatDonePayload {
                session_id: sid_str.clone(),
                prompt_tokens,
                completion_tokens,
            },
        );
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
fn get_turns(state: State<'_, AppState>, session_id: String) -> Result<Vec<TurnInfo>, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let turns = db.recent_turns(&session_id, 1000)?;
    let infos: Vec<TurnInfo> = turns
        .into_iter()
        .map(|t| TurnInfo {
            turn_num: t.turn_id.get(),
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
    session_id: String,
    title: String,
) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    db.set_session_title(&session_id, &title)?;
    drop(db);
    let _ = app.emit(
        "session-title-updated",
        SessionTitlePayload { session_id, title },
    );
    Ok(())
}

#[tauri::command]
fn delete_session(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    db.delete_session(&session_id)?;
    drop(db);
    let _ = app.emit("session-deleted", SessionDeletedPayload { session_id });
    Ok(())
}

#[tauri::command]
fn clear_all_data(app: tauri::AppHandle, state: State<'_, AppState>) -> Result<(), AppError> {
    // 1. Drop service
    *state.service.try_lock().map_err(|_| AppError::locked())? = None;

    let db_path = state.db_path.clone();

    // 2. Drop old db connection
    let placeholder = MemoryDb::open_in_memory().map_err(AppError::from)?;
    let old_db = {
        let mut guard = state.db.try_lock().map_err(|_| AppError::locked())?;
        std::mem::replace(&mut *guard, placeholder)
    };
    drop(old_db);

    // 3. Delete database files (ignore not-found errors)
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));

    // 4. Create fresh database
    let new_db = MemoryDb::open(&db_path).map_err(AppError::from)?;
    *state.db.try_lock().map_err(|_| AppError::locked())? = new_db;

    // 5. Emit event
    app.emit("data-cleared", ())
        .map_err(|e| AppError::new(ErrorKind::Internal, e.to_string()))?;

    tracing::info!("所有数据已清除，数据库已重建");
    Ok(())
}

// ── Entry point ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Open persistent database in app data directory
            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = data_dir.join("chatpm.db");

            let db = MemoryDb::open(&db_path)?;
            tracing::info!(path = %db_path.display(), "数据库已打开");

            // Try to restore API key from previous session
            let service = db
                .get_config("api_key")
                .ok()
                .flatten()
                .and_then(|raw_key| build_service(&db, &raw_key).ok());

            if service.is_some() {
                tracing::info!("已从数据库恢复 API key");
            }

            app.manage(AppState {
                db: std::sync::Mutex::new(db),
                db_path,
                service: Mutex::new(service),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            check_api_key,
            create_session,
            set_api_key,
            send_message,
            list_sessions,
            get_turns,
            update_session_title,
            delete_session,
            clear_all_data,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}
