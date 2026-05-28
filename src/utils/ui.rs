use std::io::Write;

/// Parse SEARCH / REPLACE sections from a patch string for display.
struct BlockPreview {
    search: String,
    replace: String,
}

fn parse_previews(patch: &str) -> Vec<BlockPreview> {
    let mut blocks = Vec::new();
    let mut pos = 0usize;
    let search_marker = "<<<<<<< SEARCH\n";
    let divider = "\n=======\n";
    let replace_tail = "\n>>>>>>> REPLACE";

    while let Some(m) = patch[pos..].find(search_marker) {
        let search_start = pos + m + search_marker.len();
        let div = match patch[search_start..].find(divider) {
            Some(d) => search_start + d,
            None => break,
        };
        let replace_start = div + divider.len();
        let end = match patch[replace_start..].find(replace_tail) {
            Some(e) => replace_start + e,
            None => break,
        };
        blocks.push(BlockPreview {
            search: patch[search_start..div].to_string(),
            replace: patch[replace_start..end].to_string(),
        });
        pos = end + replace_tail.len();
    }
    blocks
}

// ── Edit-Gate ──────────────────────────────────────────────────────

/// Display a SEARCH/REPLACE diff preview and ask the user for
/// confirmation before a destructive file write.
///
/// Uses `dialoguer::Confirm` for the y/n prompt.
/// Returns `true` if the user answered `y` / `yes`.
pub fn ask_user_confirmation(patch_args: &serde_json::Value, language: &str) -> bool {
    let path = patch_args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let patch = patch_args
        .get("patch")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // ── Draw box ───────────────────────────────────────────────
    println!("\n┌──────────────────────────────────────────┐");
    println!("│  EDIT-GATE  —  Confirm file change       │");
    println!("│  File: {:<34}│", truncate(path, 34));

    let blocks = parse_previews(patch);
    if blocks.is_empty() {
        println!("│  (raw patch)                              │");
        let preview = truncate(patch, 40);
        println!("│{:<42}│", preview);
    } else {
        for block in &blocks {
            println!("├──────────────────────────────────────────┤");
            println!("│  --- SEARCH ---                          │");
            for line in block.search.lines().take(6) {
                println!("│  - {:<38}│", truncate(line, 38));
            }
            if block.search.lines().count() > 6 {
                println!("│  - ... (truncated)                       │");
            }
            println!("│  +++ REPLACE +++                         │");
            for line in block.replace.lines().take(6) {
                println!("│  + {:<38}│", truncate(line, 38));
            }
            if block.replace.lines().count() > 6 {
                println!("│  + ... (truncated)                       │");
            }
        }
    }
    println!("└──────────────────────────────────────────┘");

    // Flush stdout so box is visible before dialoguer prompt.
    let _ = std::io::stdout().flush();

    let prompt = if language == "en" {
        "Do you want to apply this patch?"
    } else {
        "Apply this change?"
    };

    dialoguer::Confirm::new()
        .with_prompt(prompt)
        .default(false)
        .interact()
        .unwrap_or(false)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max.saturating_sub(3)])
    }
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_previews_single_block() {
        let patch = "<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE";
        let blocks = parse_previews(patch);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
    }

    #[test]
    fn parse_previews_multi_block() {
        let patch = "\
<<<<<<< SEARCH
fn a() {}
=======
fn a() { x(); }
>>>>>>> REPLACE
<<<<<<< SEARCH
fn b() {}
=======
fn b() { y(); }
>>>>>>> REPLACE";
        let blocks = parse_previews(patch);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].search, "fn a() {}");
        assert_eq!(blocks[1].search, "fn b() {}");
    }

    #[test]
    fn parse_previews_no_blocks() {
        let blocks = parse_previews("just some text");
        assert!(blocks.is_empty());
    }

    #[test]
    fn truncate_short() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long() {
        assert_eq!(truncate("hello world", 8), "hello...");
    }
}
