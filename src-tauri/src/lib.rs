mod commands;
mod error;
mod payload;
mod state;
mod utils;

use chat_pm_knowledge::MockEmbedder;
use chat_pm_service::knowledge::KnowledgeService;
use chat_pm_service::session::ChatService;
use chat_pm_database::ChatDb;
use std::sync::Arc;
use tauri::async_runtime;
use tauri::Manager;
use tauri_plugin_log::log;
use tokio::sync::Mutex;

use crate::error::AppError;
use crate::state::AppState;
use crate::utils::load_or_create_identity;

/// 自动从 DB 加载已存储的 API key 并尝试构建 ChatService，若有效则初始化 pipeline。
fn try_load_service(
    db: &std::sync::Mutex<ChatDb>,
) -> Option<ChatService> {
    let guard = db.lock().ok()?;
    let raw_key = guard.get_config("api_key").ok().flatten()?;
    crate::utils::build_service(&guard, &raw_key).ok()
}

fn log_targets() -> Vec<tauri_plugin_log::Target> {
    use tauri_plugin_log::{Target, TargetKind};
    let targets = vec![
        Target::new(TargetKind::Stdout),
        Target::new(TargetKind::LogDir {
            file_name: Some("chatpm.log".to_string()),
        }),
    ];
    targets
}

fn log_filter(metadata: &log::Metadata) -> bool {
    !(metadata.target().starts_with("mainline::rpc::socket")
        || (metadata.level() >= log::Level::Info
            && metadata.target().starts_with("iroh::socket::transports"))
        || metadata.target().starts_with("tracing::span"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(tauri_plugin_log::log::LevelFilter::Info)
                .filter(log_filter)
                .targets(log_targets())
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

            let service = try_load_service(&db);

            if service.is_some() {
                tracing::info!("API key restored from database");
            }

            // 初始化知识库服务
            let knowledge_stores_dir = data_dir.join("knowledge_bases");
            std::fs::create_dir_all(&knowledge_stores_dir)?;

            let embedder = Arc::new(MockEmbedder::new(384));
            let knowledge_db = {
                let guard = db.lock().map_err(|_| AppError::locked())?;
                guard.clone()
            };
            let knowledge_service = Arc::new(KnowledgeService::new(
                knowledge_db,
                embedder,
                knowledge_stores_dir.clone(),
            ));

            // 将 knowledge_service 注入到 ChatService（如果已初始化）
            let mut service = service;
            if let Some(ref mut svc) = service {
                svc.set_knowledge_service(knowledge_service.clone());
            }

            app.manage(AppState {
                db: Arc::clone(&db),
                db_path,
                service: Mutex::new(service),
                sync_engine: Mutex::new(None),
                device_id,
                knowledge_service: Mutex::new(Some(knowledge_service)),
                knowledge_stores_dir,
            });

            let handle = app.handle().clone();
            let restore_db = Arc::clone(&db);
            async_runtime::spawn(async move {
                crate::utils::restore_sync_engine(handle, restore_db).await;
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::check_api_key,
            commands::create_session,
            commands::set_api_key,
            commands::get_model,
            commands::set_model,
            commands::send_message,
            commands::list_sessions,
            commands::get_turns,
            commands::update_session_title,
            commands::delete_session,
            commands::clear_all_data,
            commands::get_sync_status,
            commands::init_and_create_sync_doc,
            commands::join_sync_doc,
            commands::stop_sync,
            commands::publish_sync_announcement,
            // 知识库命令
            commands::create_knowledge_base,
            commands::list_knowledge_bases,
            commands::rename_knowledge_base,
            commands::delete_knowledge_base,
            commands::add_kb_document,
            commands::list_kb_documents,
            commands::delete_kb_document,
            commands::search_knowledge_base,
            commands::set_session_kb_refs,
            commands::get_session_kb_refs,
        ])
        .run(tauri::generate_context!())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}
