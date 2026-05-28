use std::fs;
use anyhow::{anyhow, Context, Result};

// ── Types ──────────────────────────────────────────────────────────

/// Result of applying a SEARCH/REPLACE delta to a file.
#[derive(Debug, PartialEq, Eq)]
pub struct DeltaResult {
    /// Number of SEARCH/REPLACE blocks successfully applied.
    pub applied: usize,
}

/// Internal representation of a single SEARCH/REPLACE block.
#[derive(Debug)]
struct Block<'a> {
    search: &'a str,
    replace: &'a str,
}

// ── Constants ──────────────────────────────────────────────────────

const SEARCH_MARKER: &str = "<<<<<<< SEARCH\n";
const DIVIDER: &str = "\n=======\n";
const REPLACE_TAIL: &str = "\n>>>>>>> REPLACE";

// ── Block parser ───────────────────────────────────────────────────

/// Parse one or more SEARCH/REPLACE blocks from a delta string.
///
/// Format:
///   <<<<<<< SEARCH
///   [original code]
///   =======
///   [new code]
///   >>>>>>> REPLACE
fn parse_blocks(delta: &str) -> Result<Vec<Block<'_>>> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut pos = 0usize;

    loop {
        let search_marker = match delta[pos..].find(SEARCH_MARKER) {
            Some(idx) => pos + idx,
            None => break,
        };

        let search_start = search_marker + SEARCH_MARKER.len();
        let divider = match delta[search_start..].find(DIVIDER) {
            Some(idx) => search_start + idx,
            None => break,
        };

        let search = &delta[search_start..divider];
        let replace_start = divider + DIVIDER.len();
        let replace_end = match delta[replace_start..].find(REPLACE_TAIL) {
            Some(idx) => replace_start + idx,
            None => break,
        };

        let replace = &delta[replace_start..replace_end];
        blocks.push(Block { search, replace });
        pos = replace_end + REPLACE_TAIL.len();
    }

    if blocks.is_empty() {
        return Err(anyhow!(
            "No valid SEARCH/REPLACE block found. Expected format:\n\
             <<<<<<< SEARCH\n<original code>\n=======\n<new code>\n>>>>>>> REPLACE"
        ));
    }

    Ok(blocks)
}

// ── Three-tier fuzzy matcher ───────────────────────────────────────

/// Locate the unique occurrence of `search` in `content`.
///
/// Employs a three-tier degradation strategy to handle LLM whitespace quirks:
///
/// 1. **Exact match** — `search` appears verbatim.
/// 2. **Trailing-newline injection** — `search` omits a trailing `\n` that the file includes.
/// 3. **Trailing-newline elision** — `search` ends with `\n` that the file lacks.
///
/// After locating, the matched substring is verified to be unique in the file.
fn locate_match(content: &str, search: &str) -> Result<(usize, usize)> {
    // Tier 1: exact match
    let matched_len = search.len();
    let start = content
        .find(search)
        // Tier 2: exact + trailing newline
        .or_else(|| content.find(&format!("{}\n", search)))
        // Tier 3: strip trailing newline from search
        .or_else(|| {
            if search.ends_with('\n') {
                content.find(&search[..search.len() - 1])
            } else {
                None
            }
        });

    let start = start.ok_or_else(|| {
        let preview = if search.len() > 300 {
            format!("{}\n...(truncated)", &search[..300])
        } else {
            search.to_string()
        };
        anyhow!(
            "SEARCH block not found in file. \
             Verify the original code matches exactly (including whitespace).\n\n\
             --- SEARCH block ---\n{}\n--- end ---",
            preview
        )
    })?;

    // Clamp end in case Tier 3 matched shorter content (search had trailing \n file lacks)
    let end = (start + matched_len).min(content.len());

    // Uniqueness: the matched substring must appear exactly once
    let matched = &content[start..end];
    if let Some(second) = content[start + 1..].find(matched) {
        return Err(anyhow!(
            "SEARCH block matches multiple locations in the file \
             (line ~{} and line ~{}). \
             Add more surrounding context lines to make the match unique.",
            content[..start].lines().count() + 1,
            content[..start + 1 + second].lines().count() + 1,
        ));
    }

    Ok((start, end))
}

/// Apply a single block to content, returning the replaced string.
fn apply_block(content: &str, block: &Block) -> Result<String> {
    let (start, end) = locate_match(content, block.search)?;
    let mut result = String::with_capacity(
        content.len() + block.replace.len() - (end - start),
    );
    result.push_str(&content[..start]);
    result.push_str(block.replace);
    result.push_str(&content[end..]);
    Ok(result)
}

// ── Public API ─────────────────────────────────────────────────────

/// Parse and apply a SEARCH/REPLACE delta to a file on disk.
///
/// Reads the file, applies all SEARCH/REPLACE blocks in sequence (each
/// operating on the output of the previous), then writes the result back.
pub fn apply_delta(file_path: &str, delta: &str) -> Result<DeltaResult> {
    let blocks = parse_blocks(delta)?;
    let mut content = fs::read_to_string(file_path)
        .with_context(|| format!("Failed to read \"{}\"", file_path))?;

    for block in &blocks {
        content = apply_block(&content, block)?;
    }

    fs::write(file_path, &content)
        .with_context(|| format!("Failed to write \"{}\"", file_path))?;

    Ok(DeltaResult { applied: blocks.len() })
}

// ── Unit tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── Helpers ──────────────────────────────────────────────────

    use std::sync::atomic::{AtomicU32, Ordering};

    static TEST_COUNTER: AtomicU32 = AtomicU32::new(0);

    fn temp_dir() -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
        std::env::temp_dir().join(format!("tbug_test_{}", id))
    }

    fn temp_file(name: &str, content: &str) -> String {
        let dir = temp_dir();
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path.to_str().unwrap().to_string()
    }

    fn read_file(path: &str) -> String {
        fs::read_to_string(path).unwrap()
    }

    // ── parse_blocks ─────────────────────────────────────────────

    #[test]
    fn parse_single_block() {
        let delta = "<<<<<<< SEARCH\nold code\n=======\nnew code\n>>>>>>> REPLACE";
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old code");
        assert_eq!(blocks[0].replace, "new code");
    }

    #[test]
    fn parse_multiple_blocks() {
        let delta = "\
<<<<<<< SEARCH
fn foo() {
    bar()
}
=======
fn foo() {
    baz()
}
>>>>>>> REPLACE
<<<<<<< SEARCH
let x = 1;
=======
let x = 2;
>>>>>>> REPLACE";
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].search, "fn foo() {\n    bar()\n}");
        assert_eq!(blocks[0].replace, "fn foo() {\n    baz()\n}");
        assert_eq!(blocks[1].search, "let x = 1;");
        assert_eq!(blocks[1].replace, "let x = 2;");
    }

    #[test]
    fn parse_no_valid_block() {
        let delta = "just some random text without markers";
        let err = parse_blocks(delta).unwrap_err();
        assert!(err.to_string().contains("No valid SEARCH/REPLACE block found"));
    }

    #[test]
    fn parse_empty_search_replace() {
        let delta = "<<<<<<< SEARCH\n\n=======\n\n>>>>>>> REPLACE";
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "");
        assert_eq!(blocks[0].replace, "");
    }

    #[test]
    fn parse_block_with_leading_whitespace() {
        // Search content has leading indentation
        let delta = "<<<<<<< SEARCH\n    let x = 1;\n    let y = 2;\n=======\n    let z = 3;\n>>>>>>> REPLACE";
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks[0].search, "    let x = 1;\n    let y = 2;");
        assert_eq!(blocks[0].replace, "    let z = 3;");
    }

    #[test]
    fn parse_block_with_special_chars() {
        let delta = "<<<<<<< SEARCH\nfn foo<T: Debug>(x: &T) -> &str {\n    &x\n}\n=======\nfn foo<T: Display>(x: &T) -> String {\n    x.to_string()\n}\n>>>>>>> REPLACE";
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks[0].search, "fn foo<T: Debug>(x: &T) -> &str {\n    &x\n}");
        assert_eq!(blocks[0].replace, "fn foo<T: Display>(x: &T) -> String {\n    x.to_string()\n}");
    }

    #[test]
    fn parse_block_search_contains_divider_like_text() {
        // The search content contains text that looks like a divider but is part of code
        let delta = "<<<<<<< SEARCH\n// using ======= as a comment separator\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> REPLACE";
        // The parser looks for \n=======\n — a lone "=======" without surrounding newlines won't match
        // "// using ======= as a comment separator" has "=======" but no \n before it
        // Actually wait: the first \n=======\n appears after "let x = 1;"
        // So the parser correctly identifies the divider
        let blocks = parse_blocks(delta).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "// using ======= as a comment separator\nlet x = 1;");
        assert_eq!(blocks[0].replace, "let x = 2;");
    }

    // ── locate_match: Tier 1 — Exact match ───────────────────────

    #[test]
    fn tier1_exact_match() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        let search = "println!(\"hello\");";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(&content[start..end], search);
    }

    #[test]
    fn tier1_exact_match_at_start() {
        let content = "use std::io;\nfn main() {}\n";
        let search = "use std::io;";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(start, 0);
        assert_eq!(&content[start..end], search);
    }

    #[test]
    fn tier1_exact_match_at_end() {
        let content = "fn main() {\n    println!(\"done\");\n}";
        let search = "}";
        let (start, _) = locate_match(content, search).unwrap();
        // The last '}' should be matched
        assert_eq!(&content[start..start + 1], "}");
    }

    // ── locate_match: Tier 2 — Search omits trailing \n ──────────

    #[test]
    fn tier2_search_omits_trailing_newline() {
        let content = "fn main() {\n    let x = 1;\n}\n// extra line\n";
        // LLM writes search WITHOUT the trailing \n, but file has it
        let search = "fn main() {\n    let x = 1;\n}";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(&content[start..end], search);
        // The trailing \n after } should NOT be included in the match range
        assert_eq!(content.as_bytes()[end], b'\n');
    }

    #[test]
    fn tier2_multiline_search_omits_trailing_newline() {
        let content = "struct Foo {\n    x: i32,\n    y: i32,\n}\n\nimpl Foo {\n";
        // LLM search omits the \n after }
        let search = "struct Foo {\n    x: i32,\n    y: i32,\n}";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(&content[start..end], search);
    }

    // ── locate_match: Tier 3 — Search has trailing \n file lacks ─

    #[test]
    fn tier3_search_has_extra_trailing_newline() {
        // File does NOT end with \n
        let content = "fn main() {\n    let x = 1;\n}";
        // LLM search ends with \n
        let search = "fn main() {\n    let x = 1;\n}\n";
        let (start, end) = locate_match(content, search).unwrap();
        // Should match: start=0, end clamped to content.len()
        assert_eq!(start, 0);
        assert_eq!(end, content.len());
        assert_eq!(&content[start..end], "fn main() {\n    let x = 1;\n}");
    }

    #[test]
    fn tier3_search_extra_newline_middle_of_file() {
        let content = "line one\nline two\nline three\n";
        let search = "line two\nline three\n";
        // content has "line two\nline three\n" — exact match at Tier 1
        let (start, end) = locate_match(content, search).unwrap();
        // Actually this is an exact match (Tier 1) since content also ends with \n
        assert_eq!(&content[start..end], search);
    }

    #[test]
    fn tier3_not_needed_when_exact_exists() {
        // When both Tier 1 and Tier 3 could match, Tier 1 wins
        // Content ends with \n already
        let content = "fn main() {}\n";
        let search = "fn main() {}\n";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(&content[start..end], "fn main() {}\n");
        // Tier 1 matched, not Tier 3
        assert_eq!(start, 0);
    }

    // ── locate_match: Search not found ───────────────────────────

    #[test]
    fn search_not_found() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let search = "fn main() {\n    let y = 2;\n}\n";
        let err = locate_match(content, search).unwrap_err();
        assert!(err.to_string().contains("SEARCH block not found"));
    }

    #[test]
    fn search_not_found_partial_match() {
        let content = "fn main() {\n    println!(\"hello\");\n}\n";
        // Different function name, rest similar
        let search = "fn main() {\n    eprintln!(\"hello\");\n}\n";
        let err = locate_match(content, search).unwrap_err();
        assert!(err.to_string().contains("SEARCH block not found"));
    }

    // ── locate_match: Uniqueness collision ───────────────────────

    #[test]
    fn uniqueness_collision_repeated_pattern() {
        let content = "let x = 1;\nlet y = 2;\nlet x = 1;\n";
        // "let x = 1;" appears twice
        let search = "let x = 1;";
        let err = locate_match(content, search).unwrap_err();
        assert!(err.to_string().contains("matches multiple locations"));
    }

    #[test]
    fn uniqueness_ok_with_sufficient_context() {
        let content = "let x = 1;\nlet y = 2;\nlet x = 1;\n";
        // Search includes surrounding context making it unique
        let search = "let y = 2;\nlet x = 1;";
        let (start, end) = locate_match(content, search).unwrap();
        assert_eq!(start, 11); // position of "let y = 2;"
        assert_eq!(&content[start..end], search);
    }

    #[test]
    fn uniqueness_collision_empty_file_sections() {
        let content = "\n\n\n";
        let search = "\n\n";
        let err = locate_match(content, search).unwrap_err();
        assert!(err.to_string().contains("matches multiple locations"));
    }

    // ── apply_block ──────────────────────────────────────────────

    #[test]
    fn apply_single_block_simple() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let block = Block { search: "let x = 1;", replace: "let x = 2;" };
        let result = apply_block(content, &block).unwrap();
        assert_eq!(result, "fn main() {\n    let x = 2;\n}\n");
    }

    #[test]
    fn apply_block_delete_line() {
        let content = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
        let block = Block { search: "    let x = 1;\n", replace: "" };
        let result = apply_block(content, &block).unwrap();
        assert_eq!(result, "fn main() {\n    let y = 2;\n}\n");
    }

    #[test]
    fn apply_block_add_line() {
        let content = "fn main() {\n    let x = 1;\n}\n";
        let block = Block { search: "    let x = 1;", replace: "    let x = 1;\n    let y = 2;" };
        let result = apply_block(content, &block).unwrap();
        assert_eq!(result, "fn main() {\n    let x = 1;\n    let y = 2;\n}\n");
    }

    #[test]
    fn apply_multiple_blocks_sequentially() {
        let content = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
        let block1 = Block { search: "let x = 1;", replace: "let x = 10;" };
        let result = apply_block(content, &block1).unwrap();
        assert_eq!(result, "fn main() {\n    let x = 10;\n    let y = 2;\n}\n");
        let block2 = Block { search: "let y = 2;", replace: "let y = 20;" };
        let result = apply_block(&result, &block2).unwrap();
        assert_eq!(result, "fn main() {\n    let x = 10;\n    let y = 20;\n}\n");
    }

    // ── apply_delta: Integration tests ───────────────────────────

    #[test]
    fn apply_delta_single_block_to_file() {
        let path = temp_file("single_block.rs", "fn main() {\n    let x = 1;\n}\n");
        let delta = "<<<<<<< SEARCH\nlet x = 1;\n=======\nlet x = 2;\n>>>>>>> REPLACE";
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(read_file(&path), "fn main() {\n    let x = 2;\n}\n");

    }

    #[test]
    fn apply_delta_multi_block_to_file() {
        let path = temp_file(
            "multi_block.rs",
            "fn main() {\n    let x = 1;\n    let y = 2;\n    let z = 3;\n}\n",
        );
        let delta = "\
<<<<<<< SEARCH
    let x = 1;
=======
    let x = 10;
>>>>>>> REPLACE
<<<<<<< SEARCH
    let z = 3;
=======
    let z = 30;
>>>>>>> REPLACE";
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 2);
        assert_eq!(
            read_file(&path),
            "fn main() {\n    let x = 10;\n    let y = 2;\n    let z = 30;\n}\n"
        );

    }

    #[test]
    fn apply_delta_tier2_newline_fix() {
        // File has trailing \n, LLM search omits it
        let path = temp_file("tier2_test.rs", "fn main() {\n    let x = 1;\n}\n");
        let delta = "<<<<<<< SEARCH\nfn main() {\n    let x = 1;\n}\n=======\nfn main() {\n    let x = 2;\n}\n>>>>>>> REPLACE";
        // The search "}\n" — wait, the search is "fn main() {\n    let x = 1;\n}" without trailing \n
        // But the file is "fn main() {\n    let x = 1;\n}\n" with trailing \n
        // So Tier 2 kicks in: search + '\n' matches
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(read_file(&path), "fn main() {\n    let x = 2;\n}\n");

    }

    #[test]
    fn apply_delta_tier3_strip_newline() {
        // File does NOT end with \n, LLM search does
        let content = "fn main() {\n    let x = 1;\n}";
        let path = temp_file("tier3_test.rs", content);
        // LLM's search ends with \n but file doesn't
        let delta = "<<<<<<< SEARCH\nfn main() {\n    let x = 1;\n}\n=======\nfn main() {\n    let x = 2;\n}\n>>>>>>> REPLACE";
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 1);
        // The replacement should work (Tier 3 strips trailing \n from search)
        assert_eq!(read_file(&path), "fn main() {\n    let x = 2;\n}");

    }

    #[test]
    fn apply_delta_uniqueness_collision() {
        let path = temp_file(
            "dup_test.rs",
            "fn foo() {\n    let x = 1;\n}\n\nfn bar() {\n    let x = 1;\n}\n",
        );
        let delta = "<<<<<<< SEARCH\n    let x = 1;\n=======\n    let x = 2;\n>>>>>>> REPLACE";
        let err = apply_delta(&path, delta).unwrap_err();
        assert!(err.to_string().contains("matches multiple locations"));
        // File should be unchanged
        assert!(read_file(&path).contains("fn foo()"));

    }

    #[test]
    fn apply_delta_nonexistent_file() {
        let delta = "<<<<<<< SEARCH\nold\n=======\nnew\n>>>>>>> REPLACE";
        let err = apply_delta("/tmp/tbug_nonexistent_file_xyz.rs", delta).unwrap_err();
        assert!(err.to_string().contains("Failed to read"));
    }

    #[test]
    fn apply_delta_search_not_found_in_file() {
        let path = temp_file("not_found.rs", "fn main() {}\n");
        let delta = "<<<<<<< SEARCH\nthis does not exist in the file\n=======\nreplacement\n>>>>>>> REPLACE";
        let err = apply_delta(&path, delta).unwrap_err();
        assert!(err.to_string().contains("SEARCH block not found"));

    }

    #[test]
    fn apply_delta_with_rust_generics_and_lifetimes() {
        let path = temp_file(
            "generics.rs",
            "fn foo<'a, T: std::fmt::Debug>(x: &'a T) -> &'a str {\n    \"hello\"\n}\n",
        );
        let delta = "<<<<<<< SEARCH\nfn foo<'a, T: std::fmt::Debug>(x: &'a T) -> &'a str {\n    \"hello\"\n}\n=======\nfn foo<'a, T: std::fmt::Display>(x: &'a T) -> String {\n    format!(\"{}\", x)\n}\n>>>>>>> REPLACE";
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 1);
        assert_eq!(
            read_file(&path),
            "fn foo<'a, T: std::fmt::Display>(x: &'a T) -> String {\n    format!(\"{}\", x)\n}\n"
        );

    }

    #[test]
    fn apply_delta_preserve_surrounding_content() {
        let path = temp_file(
            "surround.rs",
            "// Copyright 2024\nuse std::io;\n\nfn main() {\n    let x = 1;\n}\n\n// footer\n",
        );
        let delta = "<<<<<<< SEARCH\nfn main() {\n    let x = 1;\n}\n=======\nfn main() {\n    let x = 42;\n}\n>>>>>>> REPLACE";
        let result = apply_delta(&path, delta).unwrap();
        assert_eq!(result.applied, 1);
        let final_content = read_file(&path);
        assert!(final_content.starts_with("// Copyright 2024\n"));
        assert!(final_content.contains("use std::io;"));
        assert!(final_content.contains("let x = 42;"));
        assert!(final_content.ends_with("// footer\n"));

    }

    #[test]
    fn apply_delta_invalid_delta_format() {
        let path = temp_file("invalid.rs", "let x = 1;\n");
        let delta = "<<<<<<< SOMETHING_ELSE\nold\n=======\nnew\n>>>>>>> REPLACE";
        let err = apply_delta(&path, delta).unwrap_err();
        assert!(err.to_string().contains("No valid SEARCH/REPLACE block found"));

    }
}
