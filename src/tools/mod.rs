pub mod file_tools;

use crate::client::ToolDefinition;

pub use file_tools::ToolResult;

// ── Tool registry ──────────────────────────────────────────────────

/// Returns the JSON Schema definitions of all built-in tools.
/// Suitable for passing directly to the LLM's `tools` parameter.
pub fn get_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        file_tools::view_file_definition(),
        file_tools::patch_file_definition(),
    ]
}

/// Dispatch a tool call by name, executing the handler and returning
/// a human-readable `ToolResult`.
///
/// Returns an error result for unknown tool names (listing available tools).
pub async fn execute_tool(name: &str, args: &serde_json::Value) -> ToolResult {
    match name {
        "view_file" => file_tools::view_file(args).await,
        "patch_file" => file_tools::patch_file(args).await,
        other => ToolResult::err(format!(
            "Unknown tool: \"{}\". Available: view_file, patch_file",
            other
        )),
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn execute_tool_unknown_returns_error_with_hint() {
        let result = execute_tool("nonexistent_tool", &serde_json::json!({})).await;
        assert!(!result.success);
        assert!(result.content.contains("Unknown tool"));
        assert!(result.content.contains("view_file"));
        assert!(result.content.contains("patch_file"));
    }

    #[tokio::test]
    async fn execute_tool_dispatches_view_file() {
        // view_file with missing path → proves the correct handler ran
        let result = execute_tool("view_file", &serde_json::json!({})).await;
        assert!(!result.success);
        assert!(result.content.contains("\"path\" parameter is required"));
    }

    #[tokio::test]
    async fn execute_tool_dispatches_patch_file() {
        // patch_file with missing args → proves the correct handler ran
        let result = execute_tool("patch_file", &serde_json::json!({})).await;
        assert!(!result.success);
        assert!(result.content.contains("\"path\" parameter is required"));
    }

    #[test]
    fn get_tool_definitions_returns_two_tools() {
        let defs = get_tool_definitions();
        assert_eq!(defs.len(), 2);
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"view_file"));
        assert!(names.contains(&"patch_file"));
    }
}
