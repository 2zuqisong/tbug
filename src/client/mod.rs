pub mod openai_types;

use std::collections::HashMap;
use std::fmt;
use std::sync::OnceLock;

use anyhow::{anyhow, Result};
use futures_util::StreamExt;

pub use openai_types::*;

// ── .env loader ──────────────────────────────────────────────────

/// Load `.env` from CWD, **overriding** existing shell environment variables.
/// Matches the TS behaviour where `.env` has highest precedence.
pub fn load_env() {
    let content = match std::fs::read_to_string(".env") {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if let Some(eq_idx) = trimmed.find('=') {
            let key = trimmed[..eq_idx].trim();
            if !key.is_empty() {
                std::env::set_var(key, trimmed[eq_idx + 1..].trim());
            }
        }
    }
}

// ── SSE parser ───────────────────────────────────────────────────

/// Internal accumulator for a single tool call whose data arrives
/// fragmented across multiple SSE deltas.
#[derive(Default, Debug)]
struct ToolCallAccum {
    id: String,
    name: String,
    arguments: String,
}

/// Minimal SSE line-buffer that ingests raw bytes and dispatches parsed
/// `StreamChunk` records through an `on_event` callback.
///
/// Decoupled from the HTTP layer so it can be unit-tested with canned data.
struct SseParser {
    buffer: String,
    content: String,
    tool_accums: HashMap<usize, ToolCallAccum>,
    usage: Option<Usage>,
}

impl SseParser {
    fn new() -> Self {
        Self {
            buffer: String::new(),
            content: String::new(),
            tool_accums: HashMap::new(),
            usage: None,
        }
    }

    /// Feed raw bytes (a single chunk from the HTTP stream) into the parser.
    /// Calls `on_event` for every complete SSE event line decoded.
    fn feed(&mut self, data: &[u8], on_event: &mut dyn FnMut(ChatStreamEvent)) {
        self.buffer
            .push_str(&String::from_utf8_lossy(data));

        // Split at '\n'; the last segment is an incomplete line → keep in buffer.
        while let Some(line_end) = self.buffer.find('\n') {
            // Extract the complete line (avoid borrowing self.buffer across
            // process_line which needs &mut self).
            let line = self.buffer[..line_end].to_string();
            self.buffer = self.buffer[line_end + 1..].to_string();
            self.process_line(&line, on_event);
        }
    }

    fn process_line(&mut self, line: &str, on_event: &mut dyn FnMut(ChatStreamEvent)) {
        let trimmed = line.trim();
        if trimmed.is_empty() || !trimmed.starts_with("data: ") {
            return;
        }

        let payload = &trimmed[6..]; // skip "data: "
        if payload == "[DONE]" {
            return;
        }

        let chunk: StreamChunk = match serde_json::from_str(payload) {
            Ok(c) => c,
            Err(_) => return, // malformed JSON – skip gracefully
        };

        let Some(choice) = chunk.choices.first() else {
            // usage-only chunks have no choices
            if let Some(u) = chunk.usage {
                self.usage = Some(Usage {
                    prompt_tokens: u.prompt_tokens,
                    completion_tokens: u.completion_tokens,
                });
            }
            return;
        };

        let delta = &choice.delta;

        if let Some(ref reasoning) = delta.reasoning_content {
            on_event(ChatStreamEvent::Thinking {
                delta: reasoning.clone(),
            });
        }

        if let Some(ref text) = delta.content {
            self.content.push_str(text);
            on_event(ChatStreamEvent::Content {
                delta: text.clone(),
            });
        }

        if let Some(ref tool_calls) = delta.tool_calls {
            for tc in tool_calls {
                let idx = tc.index.unwrap_or(0);
                let acc = self
                    .tool_accums
                    .entry(idx)
                    .or_insert_with(ToolCallAccum::default);
                if let Some(ref id) = tc.id {
                    acc.id.clone_from(id);
                }
                if let Some(ref func) = tc.function {
                    if let Some(ref name) = func.name {
                        acc.name.push_str(name);
                    }
                    if let Some(ref args) = func.arguments {
                        acc.arguments.push_str(args);
                    }
                }
            }
        }

        if let Some(u) = &chunk.usage {
            self.usage = Some(Usage {
                prompt_tokens: u.prompt_tokens,
                completion_tokens: u.completion_tokens,
            });
        }
    }

    /// Consume the parser, returning aggregated content, tool calls, and usage.
    fn finish(self) -> (String, Option<Vec<ToolCallInfo>>, Option<Usage>) {
        let tool_calls = if self.tool_accums.is_empty() {
            None
        } else {
            Some(
                self.tool_accums
                    .into_values()
                    .map(|acc| ToolCallInfo {
                        id: acc.id,
                        name: acc.name,
                        arguments: safe_json_parse(&acc.arguments),
                    })
                    .collect(),
            )
        };
        (self.content, tool_calls, self.usage)
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn safe_json_parse(raw: &str) -> serde_json::Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::Value::String(raw.to_string()))
}

// ── LLM Client ─────────────────────────────────────────────────────

pub struct LLMClient {
    api_key: String,
    base_url: String,
    http: reqwest::Client,
}

impl fmt::Debug for LLMClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LLMClient")
            .field("api_key", &"[redacted]")
            .field("base_url", &self.base_url)
            .finish()
    }
}

impl LLMClient {
    /// Create a new client.
    ///
    /// `api_key` and `base_url` can be passed explicitly, otherwise they are
    /// read from `DEEPSEEK_API_KEY` and `DEEPSEEK_API_BASE` env vars.
    pub fn new(api_key: Option<&str>, base_url: Option<&str>) -> Result<Self> {
        let api_key = api_key
            .map(String::from)
            .or_else(|| std::env::var("DEEPSEEK_API_KEY").ok())
            .filter(|k| !k.is_empty())
            .ok_or_else(|| {
                anyhow!(
                    "DEEPSEEK_API_KEY is required. \
                     Set it via environment variable, .env file, or LLMClient constructor."
                )
            })?;

        let base_url = base_url
            .map(String::from)
            .or_else(|| std::env::var("DEEPSEEK_API_BASE").ok())
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| "https://api.deepseek.com/v1".to_string());

        Ok(Self {
            api_key,
            base_url,
            http: reqwest::Client::new(),
        })
    }

    /// Send a streaming chat completion request and aggregate the response.
    ///
    /// `on_event` is called for every `ChatStreamEvent` as SSE deltas arrive,
    /// allowing the caller to print real-time progress to the terminal.
    pub async fn chat_stream(
        &self,
        messages: &[ChatMessage],
        options: Option<&ChatOptions>,
        mut on_event: impl FnMut(ChatStreamEvent),
    ) -> Result<ChatResponse> {
        let opts: ChatOptions = options.cloned().unwrap_or_default();
        let model = opts.model.as_deref().unwrap_or("deepseek-chat");

        let mut body = serde_json::json!({
            "model": model,
            "messages": messages,
            "stream": true,
        });

        if let Some(temp) = opts.temperature {
            body["temperature"] = serde_json::json!(temp);
        }
        if let Some(max_tok) = opts.max_tokens {
            body["max_tokens"] = serde_json::json!(max_tok);
        }
        if let Some(ref tools) = opts.tools {
            body["tools"] = serde_json::to_value(tools)?;
        }
        if let Some(ref tc) = opts.tool_choice {
            body["tool_choice"] = serde_json::to_value(tc)?;
        }

        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable>".to_string());
            return Err(anyhow!(
                "DeepSeek API error ({}): {}",
                status,
                error_body
            ));
        }

        let mut parser = SseParser::new();
        let mut stream = response.bytes_stream();

        while let Some(result) = stream.next().await {
            parser.feed(&result?, &mut on_event);
        }

        on_event(ChatStreamEvent::Done);

        let (content, tool_calls, usage) = parser.finish();

        Ok(ChatResponse {
            content,
            tool_calls,
            usage,
        })
    }
}

// ── Default singleton ──────────────────────────────────────────────

static DEFAULT_CLIENT: OnceLock<LLMClient> = OnceLock::new();

/// Returns a shared `LLMClient` initialised from environment variables.
///
/// Panics if `DEEPSEEK_API_KEY` is not set (this is a fatal configuration error).
pub fn get_default_client() -> &'static LLMClient {
    DEFAULT_CLIENT.get_or_init(|| {
        LLMClient::new(None, None)
            .expect("Failed to initialise default LLMClient — is DEEPSEEK_API_KEY set?")
    })
}

/// Convenience function that calls through to the default client.
#[allow(dead_code)]
pub async fn chat_stream(
    messages: &[ChatMessage],
    options: Option<&ChatOptions>,
    on_event: impl FnMut(ChatStreamEvent),
) -> Result<ChatResponse> {
    get_default_client()
        .chat_stream(messages, options, on_event)
        .await
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── safe_json_parse ─────────────────────────────────────────

    #[test]
    fn safe_json_parse_valid_object() {
        let v = safe_json_parse(r#"{"path": "src/main.rs"}"#);
        assert_eq!(v["path"], "src/main.rs");
    }

    #[test]
    fn safe_json_parse_valid_array() {
        let v = safe_json_parse(r#"["a", "b"]"#);
        assert_eq!(v[0], "a");
    }

    #[test]
    fn safe_json_parse_invalid_falls_back_to_string() {
        let v = safe_json_parse("{broken json");
        assert_eq!(v, serde_json::Value::String("{broken json".into()));
    }

    #[test]
    fn safe_json_parse_empty_string() {
        let v = safe_json_parse("");
        assert_eq!(v, serde_json::Value::String("".into()));
    }

    // ── SseParser: content stream ───────────────────────────────

    #[test]
    fn sse_content_delta_streamed() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            ChatStreamEvent::Content {
                delta: "Hello".into()
            }
        );

        let (content, tool_calls, _usage) = parser.finish();
        assert_eq!(content, "Hello");
        assert!(tool_calls.is_none());
    }

    #[test]
    fn sse_multiple_content_deltas_concatenated() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Line 1\"},\"finish_reason\":null}]}\n\
              data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"\\n\"},\"finish_reason\":null}]}\n\
              data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Line 2\"},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        assert_eq!(events.len(), 3);
        let (content, _, _) = parser.finish();
        assert_eq!(content, "Line 1\nLine 2");
    }

    #[test]
    fn sse_content_split_across_byte_chunks() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        // First chunk: partial SSE line (no newline yet)
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"He",
            &mut |e| events.push(e),
        );
        assert!(events.is_empty());

        // Second chunk: completes the line
        parser.feed(
            b"llo\"},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            ChatStreamEvent::Content {
                delta: "Hello".into()
            }
        );
    }

    // ── SseParser: thinking stream ──────────────────────────────

    #[test]
    fn sse_thinking_delta_emitted() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"I should check the file first.\"},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0],
            ChatStreamEvent::Thinking {
                delta: "I should check the file first.".into()
            }
        );

        let (content, tool_calls, _usage) = parser.finish();
        assert!(content.is_empty());
        assert!(tool_calls.is_none());
    }

    #[test]
    fn sse_thinking_interleaved_with_content() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"Let me think\"},\"finish_reason\":null}]}\n\
              data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"The fix is...\"},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        assert_eq!(events.len(), 2);
        assert_eq!(
            events[0],
            ChatStreamEvent::Thinking {
                delta: "Let me think".into()
            }
        );
        assert_eq!(
            events[1],
            ChatStreamEvent::Content {
                delta: "The fix is...".into()
            }
        );
    }

    // ── SseParser: tool call stream ─────────────────────────────

    #[test]
    fn sse_single_tool_call_fragmented_across_deltas() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        // Delta 1: id + partial name
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_abc\",\"type\":\"function\",\"function\":{\"name\":\"view_\"}}]},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );
        // Delta 2: rest of name
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"file\"}}]},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );
        // Delta 3: partial arguments
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{\\\"path\\\":\"}}]},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );
        // Delta 4: rest of arguments
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"src/main.rs\\\"}\"}}]},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        let (_content, tool_calls, _usage) = parser.finish();
        let tcs = tool_calls.unwrap();
        assert_eq!(tcs.len(), 1);
        assert_eq!(tcs[0].id, "call_abc");
        assert_eq!(tcs[0].name, "view_file");
        assert_eq!(tcs[0].arguments["path"], "src/main.rs");
    }

    #[test]
    fn sse_two_tool_calls_multiplexed() {
        let mut parser = SseParser::new();
        let mut events: Vec<ChatStreamEvent> = Vec::new();

        // Both tool calls announced in one delta
        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[\
               {\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"view_file\",\"arguments\":\"{\\\"path\\\":\\\"a.rs\\\"}\"}},\
               {\"index\":1,\"id\":\"call_2\",\"type\":\"function\",\"function\":{\"name\":\"view_file\",\"arguments\":\"{\\\"path\\\":\\\"b.rs\\\"}\"}}\
            ]},\"finish_reason\":null}]}\n",
            &mut |e| events.push(e),
        );

        let (_content, tool_calls, _usage) = parser.finish();
        let tcs = tool_calls.unwrap();
        assert_eq!(tcs.len(), 2);

        // Sort by id for deterministic assertion
        let mut tcs = tcs;
        tcs.sort_by(|a, b| a.id.cmp(&b.id));

        assert_eq!(tcs[0].id, "call_1");
        assert_eq!(tcs[0].name, "view_file");
        assert_eq!(tcs[0].arguments["path"], "a.rs");

        assert_eq!(tcs[1].id, "call_2");
        assert_eq!(tcs[1].name, "view_file");
        assert_eq!(tcs[1].arguments["path"], "b.rs");
    }

    #[test]
    fn sse_two_tool_calls_fragmented_interleaved() {
        // Real APIs interleave: tool 0 name → tool 1 name → tool 0 args → tool 1 args
        let mut parser = SseParser::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[\
               {\"index\":0,\"id\":\"a\",\"type\":\"function\",\"function\":{\"name\":\"view\"}}\
            ]},\"finish_reason\":null}]}\n\
            data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[\
               {\"index\":1,\"id\":\"b\",\"type\":\"function\",\"function\":{\"name\":\"patch\"}}\
            ]},\"finish_reason\":null}]}\n\
            data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[\
               {\"index\":0,\"function\":{\"name\":\"_file\",\"arguments\":\"{\\\"path\\\":\\\"x\\\"}\"}}\
            ]},\"finish_reason\":null}]}\n\
            data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[\
               {\"index\":1,\"function\":{\"name\":\"_file\",\"arguments\":\"{\\\"path\\\":\\\"y\\\"}\"}}\
            ]},\"finish_reason\":null}]}\n",
            &mut |_| {},
        );

        let (_content, tool_calls, _usage) = parser.finish();
        let mut tcs = tool_calls.unwrap();
        tcs.sort_by(|a, b| a.id.cmp(&b.id));

        assert_eq!(tcs[0].name, "view_file");
        assert_eq!(tcs[0].arguments["path"], "x");
        assert_eq!(tcs[1].name, "patch_file");
        assert_eq!(tcs[1].arguments["path"], "y");
    }

    #[test]
    fn sse_usage_recorded() {
        let mut parser = SseParser::new();

        parser.feed(
            b"data: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":3,\"total_tokens\":13}}\n",
            &mut |_| {},
        );

        let (_content, _tool_calls, usage) = parser.finish();
        let u = usage.unwrap();
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 3);
    }

    // ── LLMClient construction ──────────────────────────────────

    #[test]
    fn llm_client_missing_api_key() {
        let saved = std::env::var("DEEPSEEK_API_KEY").ok();
        std::env::remove_var("DEEPSEEK_API_KEY");

        let result = LLMClient::new(None, None);
        assert!(result.is_err());
        assert!(
            format!("{}", result.unwrap_err()).contains("DEEPSEEK_API_KEY")
        );

        if let Some(key) = saved {
            std::env::set_var("DEEPSEEK_API_KEY", key);
        }
    }

    #[test]
    fn llm_client_explicit_api_key_works() {
        let client = LLMClient::new(Some("sk-test"), None).unwrap();
        assert_eq!(client.api_key, "sk-test");
    }

    #[test]
    fn llm_client_default_base_url() {
        std::env::remove_var("DEEPSEEK_API_BASE");
        let client = LLMClient::new(Some("sk-test"), None).unwrap();
        assert_eq!(client.base_url, "https://api.deepseek.com/v1");
    }

    #[test]
    fn llm_client_custom_base_url() {
        let client = LLMClient::new(Some("sk-test"), Some("https://custom.api/v1")).unwrap();
        assert_eq!(client.base_url, "https://custom.api/v1");
    }
}
