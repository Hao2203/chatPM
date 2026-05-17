# chat_pm_deepseek

DeepSeek API 异步流式客户端，基于 `reqwest` + Tokio。

**职责：** API 密钥安全封装（`secrecy::SecretString`），SSE 流式响应解析，可配置模型、Token 上限与推理模式。
