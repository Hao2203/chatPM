use chat_pm_database::ChatDb;
use chat_pm_service::knowledge::KnowledgeService;
use chat_pm_service::session::ChatService;
use chat_pm_service::sync_engine::SyncEngine;
use chat_pm_sync::DeviceId;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub(crate) struct AppState {
    pub(crate) db: Arc<std::sync::Mutex<ChatDb>>,
    pub(crate) db_path: PathBuf,
    pub(crate) service: Mutex<Option<ChatService>>,
    pub(crate) sync_engine: Mutex<Option<SyncEngine>>,
    pub(crate) device_id: DeviceId,
    pub(crate) knowledge_service: Mutex<Option<Arc<KnowledgeService>>>,
    #[allow(dead_code)]
    pub(crate) knowledge_stores_dir: PathBuf,
}
