use chat_pm_commands::session::{ChatPipeline, PipelineConfig};
use chat_pm_database::MemoryDb;
use chat_pm_session::{
    message::UserInput,
    session::{NewSession, SessionId},
};
use serde::Serialize;
use tauri::{Emitter, Manager, State};
use tokio::sync::Mutex;

// ── State ───────────────────────────────────────────────────────────

struct AppState {
    db: MemoryDb,
    pipeline: Mutex<Option<ChatPipeline>>,
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
}

#[derive(Debug, Clone, Serialize)]
struct SessionTitlePayload {
    session_id: String,
    title: String,
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
}

// ── Helper ──────────────────────────────────────────────────────────

/// Try to build a ChatPipeline from a stored API key string.
fn build_pipeline(db: &MemoryDb, raw_key: &str) -> Result<ChatPipeline, String> {
    let key =
        chat_pm_deepseek::ApiKey::new(raw_key).ok_or_else(|| "invalid API key".to_string())?;
    let client = chat_pm_deepseek::Client::new(key);
    let config = PipelineConfig::default();
    ChatPipeline::new(db.clone(), client, config).map_err(|e| e.to_string())
}

// ── Commands ────────────────────────────────────────────────────────

#[tauri::command]
async fn check_api_key(state: State<'_, AppState>) -> Result<bool, String> {
    let guard = state.pipeline.lock().await;
    Ok(guard.is_some())
}

#[tauri::command]
fn create_session(state: State<'_, AppState>) -> Result<String, String> {
    let guard = state
        .pipeline
        .try_lock()
        .map_err(|_| "locked".to_string())?;
    let pipeline = guard
        .as_ref()
        .ok_or_else(|| "请先配置 API Key".to_string())?;
    let new_session = pipeline.create_session();
    Ok(new_session.session_id.to_string())
}

#[tauri::command]
fn set_api_key(state: State<'_, AppState>, api_key: String) -> Result<(), String> {
    // Validate and build pipeline
    let pipeline = build_pipeline(&state.db, &api_key)?;

    // Persist to database
    state.db.set_config("api_key", &api_key);

    // Store in memory
    let mut guard = state
        .pipeline
        .try_lock()
        .map_err(|_| "locked".to_string())?;
    *guard = Some(pipeline);

    tracing::info!("API key 已配置并持久化");
    Ok(())
}

#[tauri::command]
async fn send_message(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    session_id: String,
    content: String,
) -> Result<(), String> {
    let pipeline = {
        let guard = state.pipeline.lock().await;
        guard
            .clone()
            .ok_or_else(|| "请先配置 API Key".to_string())?
    };

    let session_id = SessionId::from_uuid(
        uuid::Uuid::parse_str(&session_id).map_err(|e| e.to_string())?,
    );
    let user_input = UserInput::new(&content);

    // ── 状态机：NewSession / Session 分流 ──────────────────────
    let session = match pipeline.resume_session(session_id) {
        // 已有标题的会话 → 直接对话
        Ok(session) => session,
        Err(_) => {
            // 无标题 → 是 NewSession，走 TitlePrompt → Session 流程
            let new_session = NewSession { session_id };
            let tp = new_session.into_title_prompt(user_input.to_string());
            let session = pipeline
                .finalize_session(tp)
                .await
                .map_err(|e| e.to_string())?;

            // Emit title event
            let _ = app.emit(
                "session-title-updated",
                SessionTitlePayload {
                    session_id: session.session_id.to_string(),
                    title: session.title.to_string(),
                },
            );

            session
        }
    };

    let mut stream = pipeline
        .chat(&session, user_input)
        .await
        .map_err(|e| e.to_string())?;

    let sid_str = session.session_id.to_string();
    let app_handle = app.clone();

    // Spawn a task to forward streaming chunks as Tauri events
    tokio::spawn(async move {
        while let Some(result) = stream.recv().await {
            match result {
                Ok(frame) => {
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
            },
        );
    });

    Ok(())
}

#[tauri::command]
fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, String> {
    let sessions = state.db.list_sessions();
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
fn get_turns(state: State<'_, AppState>, session_id: String) -> Result<Vec<TurnInfo>, String> {
    let turns = state.db.recent_turns(&session_id, 1000);
    let infos: Vec<TurnInfo> = turns
        .into_iter()
        .map(|t| TurnInfo {
            turn_num: t.turn_id.0,
            user_text: t.user_text,
            assistant_text: t.assistant_text,
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
) -> Result<(), String> {
    state.db.set_session_title(&session_id, &title);
    let _ = app.emit(
        "session-title-updated",
        SessionTitlePayload {
            session_id,
            title,
        },
    );
    Ok(())
}

// ── Entry point ─────────────────────────────────────────────────────

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            // Open persistent database in app data directory
            let data_dir = app.path().app_data_dir().expect("app data dir");
            std::fs::create_dir_all(&data_dir).expect("create data dir");
            let db_path = data_dir.join("chatpm.db");

            let db = MemoryDb::open(&db_path).expect("open database");
            tracing::info!(path = %db_path.display(), "数据库已打开");

            // Try to restore API key from previous session
            let pipeline = db
                .get_config("api_key")
                .and_then(|raw_key| build_pipeline(&db, &raw_key).ok());

            if pipeline.is_some() {
                tracing::info!("已从数据库恢复 API key");
            }

            app.manage(AppState {
                db,
                pipeline: Mutex::new(pipeline),
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
