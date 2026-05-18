use chat_pm_session::{ChatError, chat::StopReason, message::ChatMessage};
use futures_lite::StreamExt;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use serde::Deserialize;
use serde_json::json;
use tokio::sync::mpsc;

use crate::{ApiError, ApiKey, config::ReasoningEffort};

#[derive(Debug, Clone)]
pub struct ChatRequestConfig {
    pub model: String,
    pub max_tokens: usize,
    pub thinking_enabled: bool,
    pub reasoning_effort: Option<ReasoningEffort>,
}

#[derive(Debug, Clone)]
pub struct ChatChunk {
    pub raw_text: String,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
    pub stop_reason: Option<StopReason>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    api_base: String,
    api_key: ApiKey,
}

impl Client {
    pub fn new(api_key: ApiKey) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_base: "https://api.deepseek.com".to_string(),
            api_key,
        }
    }

    pub fn from_env() -> Result<Self, ChatError> {
        let api_key = std::env::var("DEEPSEEK_API_KEY")
            .map_err(|_| ChatError::ApiKeyNotConfigured)?;
        let api_key = ApiKey::new(api_key).ok_or(ChatError::InvalidApiKey)?;
        Ok(Self::new(api_key))
    }

    /// 非流式单次请求，返回完整响应文本。用于标题生成等轻量任务。
    pub async fn chat_complete(
        &self,
        request: &ChatRequestConfig,
        messages: &[ChatMessage],
    ) -> Result<String, ApiError> {
        let req_messages: Vec<_> = messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            })
            .collect();

        let mut body = json!({
            "model": request.model,
            "messages": req_messages,
            "max_tokens": request.max_tokens,
            "stream": false,
            "thinking": {
                "type": if request.thinking_enabled { "enabled" } else { "disabled" }
            }
        });

        if let Some(obj) = body.as_object_mut()
            && let Some(effort) = request.reasoning_effort
        {
            obj.insert("reasoning_effort".to_string(), json!(effort.as_str()));
        }

        let resp: ChatCompleteResponse = self
            .http
            .post(format!("{}/chat/completions", self.api_base))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose_secret()))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::RequestFailed(e.to_string()))?
            .error_for_status()
            .map_err(|e| ApiError::ErrorStatus(e.to_string()))?
            .json()
            .await
            .map_err(|e| ApiError::ParseFailed(e.to_string()))?;

        let choice = resp
            .choices
            .into_iter()
            .next()
            .ok_or(ApiError::NoChoice)?;

        Ok(choice.message.content)
    }

    pub async fn stream_chat(
        &self,
        request: &ChatRequestConfig,
        messages: &[ChatMessage],
    ) -> Result<mpsc::Receiver<Result<ChatChunk, ApiError>>, ApiError> {
        let req_messages: Vec<_> = messages
            .iter()
            .map(|m| {
                json!({
                    "role": m.role.as_str(),
                    "content": m.content,
                })
            })
            .collect();

        let mut body = json!({
            "model": request.model,
            "messages": req_messages,
            "max_tokens": request.max_tokens,
            "stream": true,
            "stream_options": { "include_usage": true },
            "thinking": {
                "type": if request.thinking_enabled { "enabled" } else { "disabled" }
            }
        });

        if let Some(obj) = body.as_object_mut()
            && let Some(effort) = request.reasoning_effort
        {
            obj.insert("reasoning_effort".to_string(), json!(effort.as_str()));
        }

        let response = self
            .http
            .post(format!("{}/chat/completions", self.api_base))
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key.expose_secret()))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| ApiError::RequestFailed(e.to_string()))?
            .error_for_status()
            .map_err(|e| ApiError::ErrorStatus(e.to_string()))?;

        let (tx, rx) = mpsc::channel(32);
        tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buf = String::new();

            while let Some(item) = stream.next().await {
                let bytes = match item {
                    Ok(b) => b,
                    Err(e) => {
                        let _ = tx.send(Err(ApiError::RequestFailed(e.to_string()))).await;
                        return;
                    }
                };

                buf.push_str(&String::from_utf8_lossy(&bytes));

                while let Some(pos) = buf.find('\n') {
                    let line = buf[..pos].trim_end_matches('\r').to_string();
                    buf.drain(..=pos);

                    if !line.starts_with("data: ") {
                        continue;
                    }

                    let data = &line[6..];
                    if data == "[DONE]" {
                        return;
                    }

                    let parsed: Result<ChatStreamResponse, _> =
                        serde_json::from_str(data).map_err(|e| ApiError::ParseFailed(e.to_string()));

                    let frame = parsed.and_then(|resp| {
                        // Usage-only chunk after streaming ends (choices empty, usage present)
                        if resp.choices.is_empty() {
                            return Ok(ChatChunk {
                                raw_text: String::new(),
                                prompt_tokens: resp.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                                completion_tokens: resp.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                                stop_reason: None,
                            });
                        }

                        let choice = resp
                            .choices
                            .into_iter()
                            .next()
                            .ok_or(ApiError::NoChoice)?;

                        let mut raw_text = String::new();
                        if let Some(reasoning) = choice.delta.reasoning_content {
                            raw_text.push_str(&reasoning);
                        }
                        if let Some(content) = choice.delta.content {
                            raw_text.push_str(&content);
                        }

                        Ok(ChatChunk {
                            raw_text,
                            prompt_tokens: resp.usage.as_ref().map(|u| u.prompt_tokens).unwrap_or(0),
                            completion_tokens: resp.usage.as_ref().map(|u| u.completion_tokens).unwrap_or(0),
                            stop_reason: choice.finish_reason.as_deref().map(map_finish_reason),
                        })
                    });

                    if tx.send(frame).await.is_err() {
                        return;
                    }
                }
            }
        });

        Ok(rx)
    }
}

#[derive(Debug, Deserialize)]
struct ChatStreamResponse {
    choices: Vec<ChatChoice>,
    usage: Option<Usage>,
}

#[derive(Debug, Deserialize)]
struct ChatCompleteResponse {
    choices: Vec<ChatCompleteChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatCompleteChoice {
    message: ChatCompleteMessage,
}

#[derive(Debug, Deserialize)]
struct ChatCompleteMessage {
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    delta: ChatDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChatDelta {
    content: Option<String>,
    reasoning_content: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Usage {
    prompt_tokens: usize,
    completion_tokens: usize,
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "length" => StopReason::MaxTokens,
        "content_filter" => StopReason::ContentFilter,
        _ => StopReason::EndOfSequence,
    }
}

trait RoleAsStr {
    fn as_str(&self) -> &'static str;
}

impl RoleAsStr for chat_pm_session::chat::Role {
    fn as_str(&self) -> &'static str {
        match self {
            chat_pm_session::chat::Role::System => "system",
            chat_pm_session::chat::Role::User => "user",
            chat_pm_session::chat::Role::Assistant => "assistant",
        }
    }
}
