mod commands;
mod error;
mod payload;
mod state;
mod utils;

use crate::state::AppState;
use crate::utils::load_or_create_identity;
use chat_pm_database::ChatDb;
use std::sync::Arc;
use tauri::async_runtime;
use tauri::Manager;
use tauri_plugin_log::log;
use tokio::sync::Mutex;

/// 自动从 DB 加载已存储的 API key 并尝试构建 ChatService，若有效则初始化 pipeline。
fn try_load_service(
    db: &std::sync::Mutex<ChatDb>,
) -> Option<chat_pm_service::session::ChatService> {
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

            app.manage(AppState {
                db: Arc::clone(&db),
                db_path,
                service: Mutex::new(service),
                sync_engine: Mutex::new(None),
                device_id,
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
        ])
        .run(tauri::generate_context!())
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    Ok(())
}
