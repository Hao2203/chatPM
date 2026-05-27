# chat_pm_database

SQLite 持久化层，基于 `rusqlite`（bundled 模式），SQLite 编译进二进制，无需外部依赖。

**职责：** 会话与对话记录的增删查改，WAL 模式，线程安全（`Arc<Mutex<Connection>>`）。
