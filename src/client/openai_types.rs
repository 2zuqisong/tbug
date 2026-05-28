use serde::{Deserialize, Serialize};

// ── Message types ──────────────────────────────────────────────────

/// OpenAI-compatible chat message sent to /v1/chat/completions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ChatMessage {
    /// `system`, `user`, `assistant`, or `tool`.
    pub role: String,
    /// Message content. `null` when the message carries only `tool_calls`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Tool calls requested by the assistant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    /// For `tool` role messages, the id of the tool call this responds to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Optional name (user name for `user` messages, tool name for `tool` messages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl ChatMessage {
    pub fn system(content: &str) -> Self {
        Self {
            role: "system".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn user(content: &str) -> Self {
        Self {
            role: "user".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            name: None,
        }
    }

    pub fn tool(tool_call_id: &str, content: &str) -> Self {
        Self {
            role: "tool".into(),
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            name: None,
        }
    }
}

// ── Tool call (assistant output) ────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub call_type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCall {
    pub name: String,
    /// JSON-encoded string of the function arguments.
    pub arguments: String,
}

// ── Tool definition (sent to LLM) ───────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolDefinition {
    #[serde(rename = "type")]
    pub def_type: String,
    pub function: FunctionDef,
}

impl ToolDefinition {
    pub fn new(name: &str, description: &str, parameters: serde_json::Value) -> Self {
        Self {
            def_type: "function".into(),
            function: FunctionDef {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

// ── Tool choice ─────────────────────────────────────────────────────

/// Controls whether the model may call tools.
///
/// Serializes as `"auto"`, `"none"`, or `{"type":"function","function":{"name":"..."}}`.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum ToolChoice {
    Auto,
    None,
    Specific { name: String },
}

impl Serialize for ToolChoice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Auto => serializer.serialize_str("auto"),
            Self::None => serializer.serialize_str("none"),
            Self::Specific { name } => {
                use serde::ser::SerializeMap;
                let mut map = serializer.serialize_map(Some(2))?;
                map.serialize_entry("type", "function")?;
                map.serialize_entry(
                    "function",
                    &serde_json::json!({ "name": name }),
                )?;
                map.end()
            }
        }
    }
}

// ── Stream event (callback) ─────────────────────────────────────────

/// Emitted by `LLMClient::chat_stream` via the `on_event` callback as
/// chunks arrive over SSE.
#[derive(Debug, Clone, PartialEq)]
pub enum ChatStreamEvent {
    /// A text content delta from the assistant.
    Content { delta: String },
    /// A reasoning / chain-of-thought delta (DeepSeek-R1 thinking).
    Thinking { delta: String },
    /// Stream has ended.
    Done,
}

// ── Chat response (final aggregated result) ─────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct ChatResponse {
    /// Aggregated text content across all deltas.
    pub content: String,
    /// Parsed tool calls (arguments deserialized to JSON).
    pub tool_calls: Option<Vec<ToolCallInfo>>,
    /// Token usage if provided by the API.
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCallInfo {
    pub id: String,
    pub name: String,
    /// Deserialized arguments (best-effort; falls back to raw string on parse failure).
    pub arguments: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
}

// ── Chat options ────────────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct ChatOptions {
    /// Model name. Defaults to `deepseek-chat`.
    pub model: Option<String>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// Available tools for the model to call.
    pub tools: Option<Vec<ToolDefinition>>,
    /// Tool-calling behaviour: auto / none / force a specific tool.
    pub tool_choice: Option<ToolChoice>,
}

// ── SSE chunk shapes (internal deserialization targets) ─────────────

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamChunk {
    pub choices: Vec<StreamChoice>,
    #[serde(default)]
    pub usage: Option<StreamUsage>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamChoice {
    pub index: u32,
    pub delta: StreamDelta,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Default, Deserialize, PartialEq)]
pub struct StreamDelta {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub reasoning_content: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamToolCallDelta {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(rename = "type", default)]
    pub call_type: Option<String>,
    #[serde(default)]
    pub function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamFunctionDelta {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct StreamUsage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── ChatMessage round-trip ──────────────────────────────────

    #[test]
    fn message_system_roundtrip() {
        let msg = ChatMessage::system("You are helpful.");
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
        assert_eq!(json, r#"{"role":"system","content":"You are helpful."}"#);
    }

    #[test]
    fn message_user_roundtrip() {
        let msg = ChatMessage::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn message_tool_roundtrip() {
        let msg = ChatMessage::tool("call_abc", "result text");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("call_abc"));
        assert!(json.contains("result text"));
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn message_with_tool_calls_roundtrip() {
        let msg = ChatMessage {
            role: "assistant".into(),
            content: None,
            tool_calls: Some(vec![ToolCall {
                id: "call_1".into(),
                call_type: "function".into(),
                function: FunctionCall {
                    name: "view_file".into(),
                    arguments: r#"{"path":"src/main.rs"}"#.into(),
                },
            }]),
            tool_call_id: None,
            name: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let back: ChatMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(msg, back);
    }

    #[test]
    fn message_content_null_when_tool_calls_present() {
        // OpenAI convention: content is null when tool_calls is present
        let json = r#"{"role":"assistant","content":null,"tool_calls":[{"id":"x","type":"function","function":{"name":"f","arguments":"{}"}}]}"#;
        let msg: ChatMessage = serde_json::from_str(json).unwrap();
        assert!(msg.content.is_none());
        assert!(msg.tool_calls.is_some());
    }

    // ── ToolChoice serialization ────────────────────────────────

    #[test]
    fn tool_choice_auto() {
        let tc = ToolChoice::Auto;
        let json = serde_json::to_string(&tc).unwrap();
        assert_eq!(json, r#""auto""#);
    }

    #[test]
    fn tool_choice_none() {
        let tc = ToolChoice::None;
        let json = serde_json::to_string(&tc).unwrap();
        assert_eq!(json, r#""none""#);
    }

    #[test]
    fn tool_choice_specific() {
        let tc = ToolChoice::Specific {
            name: "view_file".into(),
        };
        let json = serde_json::to_string(&tc).unwrap();
        assert!(json.contains(r#""type":"function""#));
        assert!(json.contains(r#""name":"view_file""#));
    }

    // ── ToolDefinition construction ─────────────────────────────

    #[test]
    fn tool_definition_builder() {
        let params = serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" }
            },
            "required": ["path"]
        });
        let def = ToolDefinition::new("view_file", "Read a file.", params);
        assert_eq!(def.def_type, "function");
        assert_eq!(def.function.name, "view_file");
        assert_eq!(def.function.description, "Read a file.");
    }

    // ── StreamChunk deserialization ─────────────────────────────

    #[test]
    fn stream_chunk_content_delta() {
        let sse = r#"{"choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("Hello"));
    }

    #[test]
    fn stream_chunk_thinking_delta() {
        let sse =
            r#"{"choices":[{"index":0,"delta":{"reasoning_content":"Let me think..."},"finish_reason":null}]}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        assert_eq!(
            chunk.choices[0].delta.reasoning_content.as_deref(),
            Some("Let me think...")
        );
    }

    #[test]
    fn stream_chunk_tool_call_id_and_name() {
        // First delta for a tool call: id + function name
        let sse = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_001","type":"function","function":{"name":"view_file","arguments":""}}]},"finish_reason":null}]}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        let tc = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(tc[0].index, Some(0));
        assert_eq!(tc[0].id.as_deref(), Some("call_001"));
        assert_eq!(tc[0].function.as_ref().unwrap().name.as_deref(), Some("view_file"));
    }

    #[test]
    fn stream_chunk_tool_call_arguments() {
        // Subsequent delta: function arguments fragment
        let sse = r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]},"finish_reason":null}]}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        let tc = chunk.choices[0].delta.tool_calls.as_ref().unwrap();
        assert_eq!(
            tc[0].function.as_ref().unwrap().arguments.as_deref(),
            Some("{\"path\":")
        );
    }

    #[test]
    fn stream_chunk_with_usage() {
        let sse = r#"{"choices":[],"usage":{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        let usage = chunk.usage.unwrap();
        assert_eq!(usage.prompt_tokens, 100);
        assert_eq!(usage.completion_tokens, 50);
    }

    #[test]
    fn stream_chunk_finish_reason_stop() {
        let sse = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
        let chunk: StreamChunk = serde_json::from_str(sse).unwrap();
        assert_eq!(
            chunk.choices[0].finish_reason.as_deref(),
            Some("stop")
        );
    }

    #[test]
    fn stream_delta_defaults_all_none() {
        let delta: StreamDelta = serde_json::from_str("{}").unwrap();
        assert!(delta.content.is_none());
        assert!(delta.reasoning_content.is_none());
        assert!(delta.tool_calls.is_none());
        assert!(delta.role.is_none());
    }

    // ── ChatStreamEvent ─────────────────────────────────────────

    #[test]
    fn chat_stream_event_variants() {
        let content = ChatStreamEvent::Content {
            delta: "hi".into(),
        };
        let thinking = ChatStreamEvent::Thinking {
            delta: "hmm".into(),
        };
        let done = ChatStreamEvent::Done;

        match content {
            ChatStreamEvent::Content { delta } => assert_eq!(delta, "hi"),
            _ => panic!("expected Content"),
        }
        match thinking {
            ChatStreamEvent::Thinking { delta } => assert_eq!(delta, "hmm"),
            _ => panic!("expected Thinking"),
        }
        assert_eq!(done, ChatStreamEvent::Done);
    }
}
