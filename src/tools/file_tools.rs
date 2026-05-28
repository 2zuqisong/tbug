use crate::client::ToolDefinition;
use crate::utils::diff::apply_delta;

// ── Tool Result ────────────────────────────────────────────────────

/// Unified return type for all tool handlers.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub success: bool,
    pub content: String,
}

impl ToolResult {
    pub fn ok(content: impl Into<String>) -> Self {
        Self {
            success: true,
            content: content.into(),
        }
    }

    pub fn err(content: impl Into<String>) -> Self {
        Self {
            success: false,
            content: content.into(),
        }
    }
}

// ── view_file ──────────────────────────────────────────────────────

/// JSON Schema definition for `view_file`.
pub fn view_file_definition() -> ToolDefinition {
    ToolDefinition::new(
        "view_file",
        "Read a file from the local filesystem. Returns content with line numbers for the specified range.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read (relative or absolute)."
                },
                "startLine": {
                    "type": "integer",
                    "description": "Starting line number (1-based, inclusive). Defaults to 1."
                },
                "endLine": {
                    "type": "integer",
                    "description": "Ending line number (1-based, inclusive). Defaults to the last line."
                }
            },
            "required": ["path"]
        }),
    )
}

/// Handler for `view_file`.
pub async fn view_file(args: &serde_json::Value) -> ToolResult {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return ToolResult::err("Error: \"path\" parameter is required."),
    };

    let start_line = args
        .get("startLine")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(1);

    let end_line = args
        .get("endLine")
        .and_then(|v| v.as_u64())
        .map(|n| n as usize)
        .unwrap_or(usize::MAX); // clamped to total lines later

    let content = match tokio::fs::read_to_string(path).await {
        Ok(c) => c,
        Err(e) => return ToolResult::err(format!("Error reading \"{}\": {}", path, e)),
    };

    let lines: Vec<&str> = content.split('\n').collect();
    let total = lines.len();
    let start = start_line.max(1);
    let end = end_line.min(total);

    if start > end {
        return ToolResult::err(format!(
            "Error: startLine ({}) > endLine ({}).",
            start, end
        ));
    }

    let selected = &lines[start - 1..end];
    let num_width = end.to_string().len();
    let output: String = selected
        .iter()
        .enumerate()
        .map(|(i, line)| {
            format!(
                "{:>width$}: {}",
                start + i,
                line,
                width = num_width
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    ToolResult::ok(output)
}

// ── patch_file ─────────────────────────────────────────────────────

/// JSON Schema definition for `patch_file`.
pub fn patch_file_definition() -> ToolDefinition {
    ToolDefinition::new(
        "patch_file",
        "Apply one or more SEARCH/REPLACE edits to a file. \
         The patch must use the format:\n\
         <<<<<<< SEARCH\n<exact original code>\n=======\n<new code>\n>>>>>>> REPLACE",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to modify."
                },
                "patch": {
                    "type": "string",
                    "description": "SEARCH/REPLACE block(s). The SEARCH section must match the file content exactly (including whitespace). Add enough surrounding context to make the match unique."
                }
            },
            "required": ["path", "patch"]
        }),
    )
}

/// Handler for `patch_file`.
///
/// Validates parameters, then delegates to the SEARCH/REPLACE diff engine
/// in `crate::utils::diff`.  File I/O is offloaded to `spawn_blocking` so
/// the async runtime stays responsive.
pub async fn patch_file(args: &serde_json::Value) -> ToolResult {
    let path = match args.get("path").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return ToolResult::err("Error: \"path\" parameter is required."),
    };

    let patch = match args.get("patch").and_then(|v| v.as_str()) {
        Some(p) if !p.is_empty() => p,
        _ => return ToolResult::err("Error: \"patch\" parameter is required."),
    };

    // clone into owned values for the spawn_blocking closure
    let path_owned = path.to_string();
    let patch_owned = patch.to_string();
    let path_for_msg = path_owned.clone();

    match tokio::task::spawn_blocking(move || apply_delta(&path_owned, &patch_owned)).await {
        Ok(Ok(result)) => ToolResult::ok(format!(
            "Successfully applied {} SEARCH/REPLACE block(s) to \"{}\".",
            result.applied, path_for_msg
        )),
        Ok(Err(e)) => ToolResult::err(format!("Error patching \"{}\": {}", path_for_msg, e)),
        Err(join_err) => ToolResult::err(format!(
            "Internal error in patch_file: {}",
            join_err
        )),
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Helpers ──────────────────────────────────────────────────

    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("tbug_tool_test_{}", id))
    }

    fn temp_file(name: &str, content: &str) -> String {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn make_args(props: &[(&str, serde_json::Value)]) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for (k, v) in props {
            map.insert(k.to_string(), v.clone());
        }
        serde_json::Value::Object(map)
    }

    // ── view_file: success paths ────────────────────────────────

    #[tokio::test]
    async fn view_file_reads_entire_file() {
        let path = temp_file("full.rs", "line one\nline two\nline three\n");
        let args = make_args(&[("path", serde_json::json!(path))]);
        let result = view_file(&args).await;
        assert!(result.success);
        assert!(result.content.contains("1: line one"));
        assert!(result.content.contains("2: line two"));
        assert!(result.content.contains("3: line three"));
    }

    #[tokio::test]
    async fn view_file_with_start_line() {
        let path = temp_file("range.rs", "a\nb\nc\nd\ne\n");
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("startLine", serde_json::json!(3)),
        ]);
        let result = view_file(&args).await;
        assert!(result.success);
        assert!(!result.content.contains("1:"));
        assert!(!result.content.contains("2:"));
        assert!(result.content.contains("3: c"));
        assert!(result.content.contains("5: e"));
    }

    #[tokio::test]
    async fn view_file_with_start_and_end_line() {
        let path = temp_file("slice.rs", "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n");
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("startLine", serde_json::json!(3)),
            ("endLine", serde_json::json!(5)),
        ]);
        let result = view_file(&args).await;
        assert!(result.success);
        let lines: Vec<&str> = result.content.split('\n').collect();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], "3: 3");
        assert_eq!(lines[2], "5: 5");
    }

    #[tokio::test]
    async fn view_file_start_line_defaults_to_one() {
        let path = temp_file("default_start.rs", "first\nsecond\n");
        let args = make_args(&[("path", serde_json::json!(path))]);
        let result = view_file(&args).await;
        assert!(result.success);
        assert!(result.content.starts_with("1: first"));
    }

    #[tokio::test]
    async fn view_file_end_line_clamped_to_last_line() {
        let path = temp_file("clamped.rs", "one\ntwo\n");
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("endLine", serde_json::json!(999)),
        ]);
        let result = view_file(&args).await;
        assert!(result.success);
        assert!(result.content.contains("2: two"));
    }

    #[tokio::test]
    async fn view_file_line_numbers_right_aligned() {
        let content = (0..150).map(|i| format!("line {}", i)).collect::<Vec<_>>().join("\n");
        let path = temp_file("align.rs", &content);
        let args = make_args(&[("path", serde_json::json!(path))]);
        let result = view_file(&args).await;
        assert!(result.success);
        // Line 5 should be "  5: line 5" (3-char width for 3-digit max)
        assert!(result.content.contains("  5:"));
        // Line 99 should be " 99: line 99"
        assert!(result.content.contains(" 99:"));
        assert!(!result.content.contains("  99:"));
        // Line 100 should be "100: line 100"
        assert!(result.content.contains("100:"));
    }

    #[tokio::test]
    async fn view_file_single_line_file() {
        let path = temp_file("single.rs", "only one line");
        let args = make_args(&[("path", serde_json::json!(path))]);
        let result = view_file(&args).await;
        assert!(result.success);
        assert_eq!(result.content, "1: only one line");
    }

    #[tokio::test]
    async fn view_file_empty_file() {
        let path = temp_file("empty.txt", "");
        let args = make_args(&[("path", serde_json::json!(path))]);
        let result = view_file(&args).await;
        assert!(result.success);
        // TS split('\n') on "" → [""] → one empty line with line number
        assert_eq!(result.content, "1: ");
    }

    // ── view_file: error paths ─────────────────────────────────

    #[tokio::test]
    async fn view_file_missing_path() {
        let args = make_args(&[]);
        let result = view_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("\"path\" parameter is required"));
    }

    #[tokio::test]
    async fn view_file_empty_path() {
        let args = make_args(&[("path", serde_json::json!(""))]);
        let result = view_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("\"path\" parameter is required"));
    }

    #[tokio::test]
    async fn view_file_nonexistent() {
        let args = make_args(&[("path", serde_json::json!("/tmp/tbug_nonexistent_file_xyz.txt"))]);
        let result = view_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("Error reading"));
    }

    #[tokio::test]
    async fn view_file_start_greater_than_end() {
        let path = temp_file("inverted.rs", "a\nb\nc\n");
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("startLine", serde_json::json!(5)),
            ("endLine", serde_json::json!(2)),
        ]);
        let result = view_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("> endLine"));
    }

    // ── view_file: JSON Schema definition ──────────────────────

    #[test]
    fn view_file_definition_has_required_path() {
        let def = view_file_definition();
        assert_eq!(def.function.name, "view_file");
        let params = &def.function.parameters;
        let required = params["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v.as_str() == Some("path")));
    }

    // ── patch_file: success paths ──────────────────────────────

    #[tokio::test]
    async fn patch_file_applies_single_block() {
        let path = temp_file("patch_ok.rs", "fn main() {\n    let x = 1;\n}\n");
        let patch = "<<<<<<< SEARCH\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> REPLACE";
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("patch", serde_json::json!(patch)),
        ]);
        let result = patch_file(&args).await;
        assert!(result.success);
        assert!(result.content.contains("Successfully applied 1"));
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, "fn main() {\n    let x = 2;\n}\n");
    }

    #[tokio::test]
    async fn patch_file_applies_multiple_blocks() {
        let path = temp_file(
            "multi_patch.rs",
            "fn main() {\n    let a = 1;\n    let b = 2;\n}\n",
        );
        let patch = "\
<<<<<<< SEARCH
    let a = 1;
=======
    let a = 10;
>>>>>>> REPLACE
<<<<<<< SEARCH
    let b = 2;
=======
    let b = 20;
>>>>>>> REPLACE";
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("patch", serde_json::json!(patch)),
        ]);
        let result = patch_file(&args).await;
        assert!(result.success);
        assert!(result.content.contains("Successfully applied 2"));
        let written = fs::read_to_string(&path).unwrap();
        assert_eq!(written, "fn main() {\n    let a = 10;\n    let b = 20;\n}\n");
    }

    // ── patch_file: error paths ────────────────────────────────

    #[tokio::test]
    async fn patch_file_missing_path() {
        let args = make_args(&[("patch", serde_json::json!("<<<<<<< SEARCH\na\n=======\nb\n>>>>>>> REPLACE"))]);
        let result = patch_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("\"path\" parameter is required"));
    }

    #[tokio::test]
    async fn patch_file_missing_patch() {
        let args = make_args(&[("path", serde_json::json!("/tmp/fake.rs"))]);
        let result = patch_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("\"patch\" parameter is required"));
    }

    #[tokio::test]
    async fn patch_file_empty_patch() {
        let args = make_args(&[
            ("path", serde_json::json!("/tmp/fake.rs")),
            ("patch", serde_json::json!("")),
        ]);
        let result = patch_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("\"patch\" parameter is required"));
    }

    #[tokio::test]
    async fn patch_file_search_not_found() {
        let path = temp_file("notfound.rs", "fn main() {}\n");
        let patch = "<<<<<<< SEARCH\nthis text is not in the file\n=======\nreplacement\n>>>>>>> REPLACE";
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("patch", serde_json::json!(patch)),
        ]);
        let result = patch_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("SEARCH block not found"));
    }

    #[tokio::test]
    async fn patch_file_invalid_delta_format() {
        let path = temp_file("invalid_fmt.rs", "let x = 1;\n");
        let patch = "this is not a valid SEARCH/REPLACE block";
        let args = make_args(&[
            ("path", serde_json::json!(path)),
            ("patch", serde_json::json!(patch)),
        ]);
        let result = patch_file(&args).await;
        assert!(!result.success);
        assert!(result.content.contains("No valid SEARCH/REPLACE block found"));
    }

    // ── patch_file: JSON Schema definition ────────────────────

    #[test]
    fn patch_file_definition_requires_path_and_patch() {
        let def = patch_file_definition();
        assert_eq!(def.function.name, "patch_file");
        let required = def.function.parameters["required"].as_array().unwrap();
        let req_names: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
        assert!(req_names.contains(&"path"));
        assert!(req_names.contains(&"patch"));
    }
}
