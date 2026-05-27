# chat_pm_session

核心领域模型 crate，定义对话系统中的纯数据结构与变换逻辑。

本 crate 不包含任何 I/O 或异步操作，仅依赖 `derive_more`。

**职责：** 消息类型、对话轮次、上下文组装、记忆管理、System Prompt 编排、多语言支持。
