// providers/copilot.rs — GitHub Copilot provider adapter.
//
// GitHub Copilot exposes an OpenAI-compatible API with special auth and
// routing headers.
//
// Chat Completions: POST https://api.githubcopilot.com/chat/completions
//
// Copilot also exposes a Responses API, but Claurst currently keeps Copilot on
// Chat Completions until Responses request/stream normalization is implemented
// end-to-end. This mirrors the OpenCode header/auth path while avoiding the
// broken hybrid /responses payload we previously sent.
//
// Required headers on model/chat requests:
//   Authorization: Bearer <github_token>
//   User-Agent: claurst/0.0.6
//   Openai-Intent: conversation-edits
//   x-initiator: user | agent
//
// Env: GITHUB_TOKEN

use std::pin::Pin;

use async_stream::stream;
use async_trait::async_trait;
use claurst_core::provider_id::{ModelId, ProviderId};
use claurst_core::types::{ContentBlock, MessageContent, Role, ToolResultContent, UsageInfo};
use futures::Stream;
use serde_json::{json, Value};
use tracing::debug;

use crate::error_handling::parse_error_response;
use crate::provider::{LlmProvider, ModelInfo};
use crate::provider_error::ProviderError;
use crate::provider_types::{
    ProviderCapabilities, ProviderRequest, ProviderResponse, ProviderStatus, StreamEvent,
    SystemPromptStyle,
};
use crate::providers::openai::OpenAiProvider;

// ---------------------------------------------------------------------------
// CopilotProvider
// ---------------------------------------------------------------------------

pub struct CopilotProvider {
    id: ProviderId,
    token: String,
    http_client: reqwest::Client,
}

impl CopilotProvider {
    pub fn new(token: String) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(600))
            .build()
            .expect("failed to build reqwest client");

        Self {
            id: ProviderId::new(ProviderId::GITHUB_COPILOT),
            token,
            http_client,
        }
    }

    pub fn from_env() -> Option<Self> {
        std::env::var("GITHUB_TOKEN").ok().map(|t| Self::new(t))
    }

    fn base_url() -> &'static str {
        "https://api.githubcopilot.com"
    }

    fn block_has_image(block: &ContentBlock) -> bool {
        match block {
            ContentBlock::Image { .. } => true,
            ContentBlock::ToolResult { content, .. } => match content {
                ToolResultContent::Text(_) => false,
                ToolResultContent::Blocks(blocks) => blocks.iter().any(Self::block_has_image),
            },
            _ => false,
        }
    }

    fn message_has_image(content: &MessageContent) -> bool {
        match content {
            MessageContent::Text(_) => false,
            MessageContent::Blocks(blocks) => blocks.iter().any(Self::block_has_image),
        }
    }

    fn request_has_image(request: &ProviderRequest) -> bool {
        request
            .messages
            .iter()
            .any(|message| Self::message_has_image(&message.content))
    }

    fn request_initiator(request: &ProviderRequest) -> &'static str {
        match request.messages.last() {
            Some(message) if message.role == Role::User => "user",
            _ => "agent",
        }
    }

    fn copilot_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(&self.token)
            .header("User-Agent", "claurst/0.0.6")
    }

    fn copilot_request_headers(
        &self,
        builder: reqwest::RequestBuilder,
        request: &ProviderRequest,
    ) -> reqwest::RequestBuilder {
        let builder = self
            .copilot_headers(builder)
            .header("Openai-Intent", "conversation-edits")
            .header("x-initiator", Self::request_initiator(request));

        if Self::request_has_image(request) {
            builder.header("Copilot-Vision-Request", "true")
        } else {
            builder
        }
    }

    fn merge_provider_options(body: &mut Value, provider_options: &Value) {
        if let Some(options) = provider_options.as_object() {
            for (key, value) in options {
                body[key] = value.clone();
            }
        }
    }

    fn map_http_error(&self, status: u16, body: &str) -> ProviderError {
        parse_error_response(status, body, &self.id)
    }

    /// Hardcoded fallback model list used when the /models endpoint is
    /// unreachable or returns empty data.
    fn hardcoded_models(provider_id: &ProviderId) -> Vec<ModelInfo> {
        vec![
            ModelInfo { id: ModelId::new("claude-sonnet-4.6"), provider_id: provider_id.clone(), name: "Claude Sonnet 4.6 (Copilot)".into(), context_window: 128_000, max_output_tokens: 32_000 },
            ModelInfo { id: ModelId::new("claude-sonnet-4.5"), provider_id: provider_id.clone(), name: "Claude Sonnet 4.5 (Copilot)".into(), context_window: 128_000, max_output_tokens: 32_000 },
            ModelInfo { id: ModelId::new("claude-haiku-4.5"), provider_id: provider_id.clone(), name: "Claude Haiku 4.5 (Copilot)".into(), context_window: 128_000, max_output_tokens: 32_000 },
            ModelInfo { id: ModelId::new("gpt-4.1"), provider_id: provider_id.clone(), name: "GPT-4.1 (Copilot)".into(), context_window: 64_000, max_output_tokens: 16_384 },
            ModelInfo { id: ModelId::new("gpt-4o"), provider_id: provider_id.clone(), name: "GPT-4o (Copilot)".into(), context_window: 128_000, max_output_tokens: 16_384 },
            ModelInfo { id: ModelId::new("gpt-4o-mini"), provider_id: provider_id.clone(), name: "GPT-4o Mini (Copilot)".into(), context_window: 128_000, max_output_tokens: 16_384 },
            ModelInfo { id: ModelId::new("gpt-5-mini"), provider_id: provider_id.clone(), name: "GPT-5 Mini (Copilot)".into(), context_window: 128_000, max_output_tokens: 128_000 },
            ModelInfo { id: ModelId::new("o3-mini"), provider_id: provider_id.clone(), name: "o3-mini (Copilot)".into(), context_window: 200_000, max_output_tokens: 100_000 },
            ModelInfo { id: ModelId::new("o4-mini"), provider_id: provider_id.clone(), name: "o4-mini (Copilot)".into(), context_window: 200_000, max_output_tokens: 100_000 },
            ModelInfo { id: ModelId::new("gemini-3-flash-preview"), provider_id: provider_id.clone(), name: "Gemini 3 Flash (Copilot)".into(), context_window: 128_000, max_output_tokens: 64_000 },
        ]
    }

    async fn send_non_streaming(
        &self,
        request: &ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        let messages = OpenAiProvider::to_openai_messages_pub(
            &request.messages,
            request.system_prompt.as_ref(),
        );
        let tools = OpenAiProvider::to_openai_tools_pub(&request.tools);

        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
            "stream": false,
            "store": false,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(t) = request.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = request.top_p {
            body["top_p"] = json!(p);
        }
        if !request.stop_sequences.is_empty() {
            body["stop"] = json!(request.stop_sequences);
        }
        Self::merge_provider_options(&mut body, &request.provider_options);

        let url = format!("{}/chat/completions", Self::base_url());

        let builder = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json");
        let builder = self.copilot_request_headers(builder, request);

        let resp = builder
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("HTTP request failed: {}", e),
                status: None,
                body: None,
            })?;

        let status = resp.status().as_u16();
        let text = resp.text().await.map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Failed to read response body: {}", e),
            status: Some(status),
            body: None,
        })?;

        if !(200..300).contains(&(status as usize)) {
            return Err(self.map_http_error(status, &text));
        }

        let json_val: Value = serde_json::from_str(&text).map_err(|e| ProviderError::Other {
            provider: self.id.clone(),
            message: format!("Failed to parse response JSON: {}", e),
            status: Some(status),
            body: Some(text.clone()),
        })?;

        OpenAiProvider::parse_non_streaming_response_pub(&json_val, &self.id)
    }

    async fn do_streaming(
        &self,
        request: &ProviderRequest,
    ) -> Result<reqwest::Response, ProviderError> {
        let messages = OpenAiProvider::to_openai_messages_pub(
            &request.messages,
            request.system_prompt.as_ref(),
        );
        let tools = OpenAiProvider::to_openai_tools_pub(&request.tools);

        let mut body = json!({
            "model": request.model,
            "max_tokens": request.max_tokens,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
            "store": false,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools);
        }
        if let Some(t) = request.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(p) = request.top_p {
            body["top_p"] = json!(p);
        }
        if !request.stop_sequences.is_empty() {
            body["stop"] = json!(request.stop_sequences);
        }
        Self::merge_provider_options(&mut body, &request.provider_options);

        let url = format!("{}/chat/completions", Self::base_url());

        let builder = self
            .http_client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream");
        let builder = self.copilot_request_headers(builder, request);

        let resp = builder
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Other {
                provider: self.id.clone(),
                message: format!("HTTP request failed: {}", e),
                status: None,
                body: None,
            })?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&(status as usize)) {
            let text = resp.text().await.unwrap_or_default();
            return Err(self.map_http_error(status, &text));
        }

        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// LlmProvider impl
// ---------------------------------------------------------------------------

#[async_trait]
impl LlmProvider for CopilotProvider {
    fn id(&self) -> &ProviderId {
        &self.id
    }

    fn name(&self) -> &str {
        "GitHub Copilot"
    }

    async fn create_message(
        &self,
        request: ProviderRequest,
    ) -> Result<ProviderResponse, ProviderError> {
        self.send_non_streaming(&request).await
    }

    async fn create_message_stream(
        &self,
        request: ProviderRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, ProviderError>> + Send>>, ProviderError>
    {
        let resp = self.do_streaming(&request).await?;
        let provider_id = self.id.clone();

        let s = stream! {
            use futures::StreamExt;

            let mut byte_stream = resp.bytes_stream();
            let mut leftover = String::new();

            let mut message_started = false;
            let mut message_id = String::from("unknown");
            let mut model_name = String::new();
            let mut tool_call_buffers: std::collections::HashMap<
                usize,
                (String, String, String),
            > = std::collections::HashMap::new();

            while let Some(chunk_result) = byte_stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        yield Err(ProviderError::StreamError {
                            provider: provider_id.clone(),
                            message: format!("Stream read error: {}", e),
                            partial_response: None,
                        });
                        return;
                    }
                };

                let text = String::from_utf8_lossy(&chunk);
                let combined = if leftover.is_empty() {
                    text.to_string()
                } else {
                    let mut s = std::mem::take(&mut leftover);
                    s.push_str(&text);
                    s
                };

                let mut lines: Vec<&str> = combined.split('\n').collect();
                if !combined.ends_with('\n') {
                    leftover = lines.pop().unwrap_or("").to_string();
                }

                for line in lines {
                    let line = line.trim_end_matches('\r').trim();

                    if line.is_empty() || line.starts_with(':') {
                        continue;
                    }

                    let data = if let Some(rest) = line.strip_prefix("data:") {
                        rest.trim()
                    } else {
                        continue;
                    };

                    if data == "[DONE]" {
                        yield Ok(StreamEvent::MessageStop);
                        return;
                    }

                    let chunk_json: Value = match serde_json::from_str(data) {
                        Ok(v) => v,
                        Err(e) => {
                            debug!("Failed to parse Copilot SSE chunk: {}: {}", e, data);
                            continue;
                        }
                    };

                    if !message_started {
                        if let Some(id) = chunk_json.get("id").and_then(|v| v.as_str()) {
                            message_id = id.to_string();
                        }
                        if let Some(m) = chunk_json.get("model").and_then(|v| v.as_str()) {
                            model_name = m.to_string();
                        }
                        yield Ok(StreamEvent::MessageStart {
                            id: message_id.clone(),
                            model: model_name.clone(),
                            usage: UsageInfo::default(),
                        });
                        yield Ok(StreamEvent::ContentBlockStart {
                            index: 0,
                            content_block: ContentBlock::Text { text: String::new() },
                        });
                        message_started = true;
                    }

                    let choices = match chunk_json.get("choices").and_then(|c| c.as_array()) {
                        Some(c) => c,
                        None => {
                            if let Some(usage_val) = chunk_json.get("usage") {
                                let usage = OpenAiProvider::parse_usage_pub(Some(usage_val));
                                yield Ok(StreamEvent::MessageDelta {
                                    stop_reason: None,
                                    usage: Some(usage),
                                });
                            }
                            continue;
                        }
                    };

                    let choice = match choices.first() {
                        Some(c) => c,
                        None => continue,
                    };

                    let delta = match choice.get("delta") {
                        Some(d) => d,
                        None => continue,
                    };

                    if let Some(content) = delta.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            yield Ok(StreamEvent::TextDelta {
                                index: 0,
                                text: content.to_string(),
                            });
                        }
                    }

                    if let Some(tool_calls) =
                        delta.get("tool_calls").and_then(|t| t.as_array())
                    {
                        for tc in tool_calls {
                            let tc_index = tc
                                .get("index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize;
                            if let Some(tc_id) = tc.get("id").and_then(|v| v.as_str()) {
                                let name = tc
                                    .get("function")
                                    .and_then(|f| f.get("name"))
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let block_index = 1 + tc_index;
                                tool_call_buffers.insert(
                                    block_index,
                                    (tc_id.to_string(), name.clone(), String::new()),
                                );
                                yield Ok(StreamEvent::ContentBlockStart {
                                    index: block_index,
                                    content_block: ContentBlock::ToolUse {
                                        id: tc_id.to_string(),
                                        name,
                                        input: serde_json::json!({}),
                                    },
                                });
                            }
                            if let Some(args_frag) = tc
                                .get("function")
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                            {
                                if !args_frag.is_empty() {
                                    let block_index = 1 + tc_index;
                                    if let Some((_, _, buf)) =
                                        tool_call_buffers.get_mut(&block_index)
                                    {
                                        buf.push_str(args_frag);
                                    }
                                    yield Ok(StreamEvent::InputJsonDelta {
                                        index: block_index,
                                        partial_json: args_frag.to_string(),
                                    });
                                }
                            }
                        }
                    }

                    if let Some(finish_reason) =
                        choice.get("finish_reason").and_then(|v| v.as_str())
                    {
                        if !finish_reason.is_empty() && finish_reason != "null" {
                            yield Ok(StreamEvent::ContentBlockStop { index: 0 });
                            let mut tc_indices: Vec<usize> =
                                tool_call_buffers.keys().cloned().collect();
                            tc_indices.sort();
                            for idx in tc_indices {
                                yield Ok(StreamEvent::ContentBlockStop { index: idx });
                            }

                            let stop_reason =
                                OpenAiProvider::map_finish_reason_pub(finish_reason);
                            let usage_val = chunk_json.get("usage");
                            let usage =
                                usage_val.map(|u| OpenAiProvider::parse_usage_pub(Some(u)));

                            yield Ok(StreamEvent::MessageDelta {
                                stop_reason: Some(stop_reason),
                                usage,
                            });
                        }
                    }
                }
            }

            if message_started {
                yield Ok(StreamEvent::MessageStop);
            }
        };

        Ok(Box::pin(s))
    }

    async fn list_models(&self) -> Result<Vec<ModelInfo>, ProviderError> {
        // Try to fetch the live model list from the Copilot API.
        let url = format!("{}/models", Self::base_url());
        let builder = self.http_client.get(&url);
        let builder = self.copilot_headers(builder)
            .header("Accept", "application/json");

        let resp = builder.send().await;

        match resp {
            Ok(r) if r.status().is_success() => {
                let text = r.text().await.map_err(|e| ProviderError::Other {
                    provider: self.id.clone(),
                    message: e.to_string(),
                    status: None,
                    body: None,
                })?;
                let json: Value = serde_json::from_str(&text).map_err(|e| ProviderError::Other {
                    provider: self.id.clone(),
                    message: format!("Failed to parse models JSON: {}", e),
                    status: None,
                    body: Some(text.clone()),
                })?;

                let mut models = Vec::new();

                // The Copilot /models endpoint may return { "data": [...] } or
                // a top-level array.
                let items: Option<&Vec<Value>> = json
                    .get("data")
                    .and_then(|d| d.as_array())
                    .or_else(|| json.as_array());

                if let Some(arr) = items {
                    for item in arr {
                        if item
                            .get("model_picker_enabled")
                            .and_then(|v| v.as_bool())
                            == Some(false)
                        {
                            continue;
                        }
                        if let Some(endpoints) =
                            item.get("supported_endpoints").and_then(|v| v.as_array())
                        {
                            let supports_chat = endpoints.iter().any(|endpoint| {
                                endpoint
                                    .as_str()
                                    .map(|value| value.contains("chat/completions"))
                                    .unwrap_or(false)
                            });
                            if !supports_chat {
                                continue;
                            }
                        }
                        if let Some(id) = item.get("id").and_then(|v| v.as_str()) {
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or(id);
                            let ctx = item
                                .get("context_window")
                                .or_else(|| item.get("capabilities").and_then(|c| c.get("limits").and_then(|l| l.get("max_context_window_tokens"))))
                                .or_else(|| item.get("capabilities").and_then(|c| c.get("limits").and_then(|l| l.get("max_prompt_tokens"))))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(128_000) as u32;
                            let max_out = item
                                .get("max_output_tokens")
                                .or_else(|| item.get("capabilities").and_then(|c| c.get("limits").and_then(|l| l.get("max_output_tokens"))))
                                .and_then(|v| v.as_u64())
                                .unwrap_or(16_384) as u32;
                            models.push(ModelInfo {
                                id: ModelId::new(id),
                                provider_id: self.id.clone(),
                                name: name.to_string(),
                                context_window: ctx,
                                max_output_tokens: max_out,
                            });
                        }
                    }
                }

                if !models.is_empty() {
                    Ok(models)
                } else {
                    // API returned but no usable models — fall back to hardcoded.
                    Ok(Self::hardcoded_models(&self.id))
                }
            }
            _ => {
                // Network error or non-success status — fall back to hardcoded.
                Ok(Self::hardcoded_models(&self.id))
            }
        }
    }

    async fn health_check(&self) -> Result<ProviderStatus, ProviderError> {
        let url = format!("{}/models", Self::base_url());
        let builder = self.http_client.get(&url);
        let builder = self.copilot_headers(builder);

        let resp = builder.send().await;
        match resp {
            Ok(r) if r.status().is_success() => Ok(ProviderStatus::Healthy),
            Ok(r) if r.status().as_u16() == 401 || r.status().as_u16() == 403 => {
                Ok(ProviderStatus::Unavailable {
                    reason: "authentication failed — check GITHUB_TOKEN".to_string(),
                })
            }
            Ok(r) => Ok(ProviderStatus::Degraded {
                reason: format!("models endpoint returned {}", r.status()),
            }),
            Err(e) => Ok(ProviderStatus::Unavailable {
                reason: e.to_string(),
            }),
        }
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            streaming: true,
            tool_calling: true,
            thinking: false,
            image_input: true,
            pdf_input: false,
            audio_input: false,
            video_input: false,
            caching: false,
            structured_output: true,
            system_prompt_style: SystemPromptStyle::SystemMessage,
        }
    }
}
