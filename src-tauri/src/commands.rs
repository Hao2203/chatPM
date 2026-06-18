use crate::error::AppError;
use crate::error::ErrorKind;
use crate::payload::{
    ChatChunkPayload, ChatDonePayload, KbDocInfo, KbInfo, KbSearchResult,
    SessionDeletedPayload, SessionInfo, SessionTitlePayload,
    SyncStatusPayload, TurnInfo, DEFAULT_MODEL, SUPPORTED_MODELS,
};
use crate::state::AppState;
use crate::utils::{build_service, bytes_to_hex};
use chat_pm_service::knowledge::KnowledgeService;
use chat_pm_service::session::CommandError;
use chat_pm_service::sync_engine::{SyncConfig, SyncEngine, SyncTicket};
use chat_pm_database::ChatDb;
use chat_pm_knowledge::{KnowledgeBaseId, KnowledgeBaseName};
use chat_pm_session::message::UserInput;
use chat_pm_session::session::{NewSession, SessionId};
use chat_pm_session::ChatError;
use chat_pm_sync::TurnSnapshot;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Instant;
use tauri::{Emitter, Manager, State};
use tracing::info;

// ── 核心聊天命令 ────────────────────────────────────────────────────

#[tauri::command]
pub(crate) async fn check_api_key(state: State<'_, AppState>) -> Result<bool, AppError> {
    let guard = state.service.lock().await;
    Ok(guard.is_some())
}

#[tauri::command]
pub(crate) fn create_session(state: State<'_, AppState>) -> Result<String, AppError> {
    let guard = state.service.try_lock().map_err(|_| AppError::locked())?;
    let service = guard.as_ref().ok_or_else(AppError::not_configured)?;
    let new_session = service.create_session()?;
    Ok(new_session.session_id().to_string())
}

#[tauri::command]
pub(crate) fn set_api_key(state: State<'_, AppState>, api_key: String) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let service = build_service(&db, &api_key)?;
    db.set_config("api_key", &api_key)?;
    drop(db);

    let mut guard = state.service.try_lock().map_err(|_| AppError::locked())?;
    *guard = Some(service);

    info!("API key configured and persisted");
    Ok(())
}

#[tauri::command]
pub(crate) fn get_model(state: State<'_, AppState>) -> Result<String, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    Ok(db
        .get_config("model")
        .ok()
        .flatten()
        .unwrap_or_else(|| DEFAULT_MODEL.to_string()))
}

#[tauri::command]
pub(crate) fn set_model(state: State<'_, AppState>, model: &str) -> Result<(), AppError> {
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

    if let Some(raw_key) = db.get_config("api_key").ok().flatten() {
        let service = build_service(&db, &raw_key)?;
        drop(db);
        let mut guard = state.service.try_lock().map_err(|_| AppError::locked())?;
        *guard = Some(service);
    } else {
        drop(db);
    }

    info!(%model, "Model switched");
    Ok(())
}

#[tauri::command]
pub(crate) async fn send_message(
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

    let session = match service.resume_session(session_id) {
        Ok(session) => session,
        Err(CommandError::Chat(
            ChatError::SessionNotFound(_) | ChatError::TitleNotGenerated(_),
        )) => {
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

        let app_state = app_handle.state::<AppState>();
        let turn_snapshot = {
            let db_guard = match app_state.db.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            match db_guard.recent_turns(sid, 1) {
                Ok(turns) => turns.first().map(|t| TurnSnapshot {
                    turn_id: t.turn_id,
                    session_id: t.session_id,
                    turn_num: t.turn_num,
                    user_text: t.user_text.clone(),
                    assistant_text: t.assistant_text.clone(),
                    created_at: t.created_at,
                    device_id: t.device_id.unwrap_or(app_state.device_id),
                }),
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
pub(crate) fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionInfo>, AppError> {
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
pub(crate) fn get_turns(
    state: State<'_, AppState>,
    session_id: &str,
) -> Result<Vec<TurnInfo>, AppError> {
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
pub(crate) fn update_session_title(
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
pub(crate) fn delete_session(
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
pub(crate) fn clear_all_data(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    *state.service.try_lock().map_err(|_| AppError::locked())? = None;

    let placeholder = ChatDb::open_in_memory()?;
    let old_db = {
        let mut guard = state.db.try_lock().map_err(|_| AppError::locked())?;
        std::mem::replace(&mut *guard, placeholder)
    };
    drop(old_db);

    let _ = std::fs::remove_file(&state.db_path);
    let _ = std::fs::remove_file(state.db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(state.db_path.with_extension("db-shm"));

    let new_db = ChatDb::open(&state.db_path)?;
    *state.db.try_lock().map_err(|_| AppError::locked())? = new_db;

    app.emit("data-cleared", ())
        .map_err(|e| AppError::new(ErrorKind::Internal, e.to_string()))?;

    info!("All data cleared, database rebuilt");
    Ok(())
}

// ── 同步命令 ────────────────────────────────────────────────────────

#[tauri::command]
pub(crate) async fn get_sync_status(
    state: State<'_, AppState>,
) -> Result<SyncStatusPayload, AppError> {
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
pub(crate) async fn init_and_create_sync_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<String, AppError> {
    let db = Arc::clone(&state.db);

    let engine = SyncEngine::create(db.clone(), SyncConfig::default(), None).await?;
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

    let _ = app.emit(
        "sync-status-changed",
        SyncStatusPayload {
            status: "syncing".to_string(),
            active: true,
            ticket: None,
        },
    );

    Ok(ticket)
}

#[tauri::command]
pub(crate) async fn join_sync_doc(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    ticket: String,
) -> Result<(), AppError> {
    let db = Arc::clone(&state.db);
    let doc_ticket = SyncTicket::from_str(&ticket)?;

    let engine = SyncEngine::join(db.clone(), SyncConfig::default(), None, doc_ticket).await?;
    let secret_key_bytes = engine.secret_key_bytes();

    {
        let guard = db.lock().map_err(|_| AppError::locked())?;
        guard.set_config("sync_state", "active")?;
        guard.set_config("sync_role", "joiner")?;
        guard.set_config("sync_ticket", &ticket)?;
        guard.set_config("sync_secret_key", &bytes_to_hex(&secret_key_bytes))?;
    }

    *state.sync_engine.lock().await = Some(engine);

    let _ = app.emit(
        "sync-status-changed",
        SyncStatusPayload {
            status: "syncing".to_string(),
            active: true,
            ticket: None,
        },
    );

    Ok(())
}

#[tauri::command]
pub(crate) async fn stop_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<(), AppError> {
    let mut guard = state.sync_engine.lock().await;
    let _old_engine = guard.take();

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
pub(crate) async fn publish_sync_announcement(state: State<'_, AppState>) -> Result<(), AppError> {
    let guard = state.sync_engine.lock().await;
    let engine = guard
        .as_ref()
        .ok_or_else(|| AppError::new(ErrorKind::Validation, "同步引擎未启动"))?;
    engine.handle_neighbor_up(Instant::now());
    Ok(())
}

// ── 知识库命令 ──────────────────────────────────────────────────────

/// 获取知识库服务的辅助函数。
async fn get_knowledge_service(state: &State<'_, AppState>) -> Result<Arc<KnowledgeService>, AppError> {
    let guard = state.knowledge_service.lock().await;
    guard
        .as_ref()
        .cloned()
        .ok_or_else(|| AppError::new(ErrorKind::Validation, "知识库服务未初始化"))
}

#[tauri::command]
pub(crate) async fn create_knowledge_base(
    state: State<'_, AppState>,
    name: String,
) -> Result<KbInfo, AppError> {
    let ks = get_knowledge_service(&state).await?;
    let kb_name = KnowledgeBaseName::new(&name);
    let kb = ks.create_kb(&kb_name).await?;
    Ok(KbInfo {
        kb_id: kb.id.to_string(),
        name: kb.name.into_inner(),
        created_at: chrono::Utc::now().to_rfc3339(),
        document_count: kb.document_count,
        total_chunks: kb.total_chunks,
    })
}

#[tauri::command]
pub(crate) async fn list_knowledge_bases(state: State<'_, AppState>) -> Result<Vec<KbInfo>, AppError> {
    let ks = get_knowledge_service(&state).await?;
    let records = ks.list_kbs().await?;
    Ok(records
        .into_iter()
        .map(|r| KbInfo {
            kb_id: r.kb_id.to_string(),
            name: r.name,
            created_at: r.created_at.to_rfc3339(),
            document_count: r.document_count,
            total_chunks: r.total_chunks,
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn rename_knowledge_base(
    state: State<'_, AppState>,
    kb_id: String,
    new_name: String,
) -> Result<(), AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let name = KnowledgeBaseName::new(&new_name);
    ks.rename_kb(id, &name).await?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn delete_knowledge_base(
    state: State<'_, AppState>,
    kb_id: String,
) -> Result<(), AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    ks.delete_kb(id).await?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn add_kb_document(
    state: State<'_, AppState>,
    kb_id: String,
    title: String,
    text: String,
) -> Result<KbDocInfo, AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let doc = ks.add_document(id, title.clone(), &text).await?;
    Ok(KbDocInfo {
        doc_id: doc.doc_id,
        kb_id: doc.kb_id.to_string(),
        title: doc.title,
        chunk_count: doc.chunk_count,
        char_count: doc.char_count,
        created_at: doc.created_at.to_rfc3339(),
    })
}

#[tauri::command]
pub(crate) async fn list_kb_documents(
    state: State<'_, AppState>,
    kb_id: String,
) -> Result<Vec<KbDocInfo>, AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let docs = ks.list_documents(id).await?;
    Ok(docs
        .into_iter()
        .map(|d| KbDocInfo {
            doc_id: d.doc_id,
            kb_id: d.kb_id.to_string(),
            title: d.title,
            chunk_count: d.chunk_count,
            char_count: d.char_count,
            created_at: d.created_at.to_rfc3339(),
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn delete_kb_document(
    state: State<'_, AppState>,
    kb_id: String,
    doc_id: String,
) -> Result<(), AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    ks.delete_document(id, &doc_id).await?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn search_knowledge_base(
    state: State<'_, AppState>,
    kb_id: String,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<KbSearchResult>, AppError> {
    let ks = get_knowledge_service(&state).await?;
    let id: KnowledgeBaseId = kb_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let results = ks.hybrid_search(id, &query, limit.unwrap_or(5)).await?;
    Ok(results
        .into_iter()
        .map(|r| KbSearchResult {
            chunk_id: r.chunk_id,
            document_id: r.document_id,
            chunk_index: r.chunk_index,
            content: r.content,
            score: r.score,
        })
        .collect())
}

#[tauri::command]
pub(crate) async fn set_session_kb_refs(
    state: State<'_, AppState>,
    session_id: String,
    kb_ids: Vec<String>,
) -> Result<(), AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sid: SessionId = session_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let ids: Vec<KnowledgeBaseId> = kb_ids.iter().filter_map(|s| s.parse().ok()).collect();
    db.set_session_kb_refs(sid, &ids)?;
    Ok(())
}

#[tauri::command]
pub(crate) async fn get_session_kb_refs(
    state: State<'_, AppState>,
    session_id: String,
) -> Result<Vec<String>, AppError> {
    let db = state.db.try_lock().map_err(|_| AppError::locked())?;
    let sid: SessionId = session_id
        .parse()
        .map_err(|e: uuid::Error| AppError::new(ErrorKind::Validation, e.to_string()))?;
    let ids = db.get_session_kb_refs(sid)?;
    Ok(ids.into_iter().map(|id| id.to_string()).collect())
}
