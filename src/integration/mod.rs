use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Default tbug configuration directory name, placed under the user's home.
pub const TBUG_DIR_NAME: &str = ".tbug";

// ── Shell hook scripts ────────────────────────────────────────────────

/// ZSH preexec / precmd hook.
///
/// On `preexec` the raw command line is saved into `$TBUG_LAST_RAW_CMD`.
/// On `precmd`, if the exit code is non-zero and the command does not
/// start with `tb`, the command is written to `~/.tbug/last_cmd.log`.
pub const ZSH_HOOK_SCRIPT: &str = r#"
tbug_preexec() {
    export TBUG_LAST_RAW_CMD="$1"
}

tbug_precmd() {
    local exit_code=$?
    if [ $exit_code -ne 0 ] && [[ "$TBUG_LAST_RAW_CMD" != tb* ]]; then
        echo "$TBUG_LAST_RAW_CMD" > ~/.tbug/last_cmd.log
    fi
}

preexec_functions+=(tbug_preexec)
precmd_functions+=(tbug_precmd)
"#;

/// Bash hook using `history 1` in PROMPT_COMMAND.
///
/// Bash has no native `preexec`; we use `history 1` to recover the last
/// command when the prompt is about to be displayed.  Commands prefixed
/// with `tb` are skipped so `tbug` itself is never captured.
pub const BASH_HOOK_SCRIPT: &str = r#"
tbug_prompt_cmd() {
    local ec=$?
    if [ $ec -ne 0 ]; then
        local last_cmd
        last_cmd=$(history 1 2>/dev/null | sed 's/^[[:space:]]*[0-9]\+[[:space:]]*//')
        case "$last_cmd" in
            tb*|"") ;;
            *) echo "$last_cmd" > ~/.tbug/last_cmd.log ;;
        esac
    fi
}

PROMPT_COMMAND="${PROMPT_COMMAND:+$PROMPT_COMMAND;}tbug_prompt_cmd"
"#;

/// Fish hook using `fish_preexec` / `fish_postexec` events.
///
/// The raw command (`$argv`) is saved on `fish_preexec`.  On
/// `fish_postexec` the exit status is checked and, when non-zero,
/// commands not matching `tb*` are written to `~/.tbug/last_cmd.log`.
pub const FISH_HOOK_SCRIPT: &str = r#"
function tbug_preexec --on-event fish_preexec
    set -g TBUG_LAST_RAW_CMD $argv
end

function tbug_postexec --on-event fish_postexec
    if test $status -ne 0
        if not string match -q "tb*" "$TBUG_LAST_RAW_CMD"
            echo "$TBUG_LAST_RAW_CMD" > ~/.tbug/last_cmd.log
        end
    end
end
"#;

// ── Public API ────────────────────────────────────────────────────────

/// Returns the path to `$HOME/.tbug`, creating the directory if it doesn't exist.
pub fn get_tbug_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let path = PathBuf::from(home).join(TBUG_DIR_NAME);
    if !path.exists() {
        let _ = fs::create_dir_all(&path);
    }
    path
}

/// Idempotently append `hook_content` to `config_path`, guarded by a
/// `# === TBUG HOOK ===` sentinel so repeated runs never duplicate the block.
///
/// If `config_path` does not exist the call is a silent no-op (the shell
/// isn't installed).
pub fn inject_hook_to_file(config_path: &Path, hook_content: &str) -> Result<()> {
    if !config_path.exists() {
        return Ok(());
    }

    let content = fs::read_to_string(config_path)
        .with_context(|| format!("Failed to read {}", config_path.display()))?;

    if content.contains("# === TBUG HOOK ===") {
        return Ok(());
    }

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(config_path)
        .with_context(|| format!("Failed to open {} for append", config_path.display()))?;

    // Ensure the hook block starts on its own line.
    if !content.is_empty() && !content.ends_with('\n') {
        writeln!(file)?;
    }
    writeln!(file)?;
    writeln!(file, "# === TBUG HOOK ===")?;
    write!(file, "{}", hook_content)?;
    writeln!(file, "# === END TBUG HOOK ===")?;

    Ok(())
}

/// Read the last failed command from `$HOME/.tbug/last_cmd.log`.
///
/// Returns `None` when the file is missing or its content is empty / blank.
pub fn read_last_command() -> Option<String> {
    let path = get_tbug_home().join("last_cmd.log");
    match fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        }
        Err(_) => None,
    }
}

/// Read the last error ANSI capture from `$HOME/.tbug/last_error.ansi`.
///
/// Raw bytes are preserved so ANSI escape sequences survive the round-trip.
/// Returns an empty string when the file does not exist.
pub fn read_last_error() -> String {
    let path = get_tbug_home().join("last_error.ansi");
    match fs::read(&path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(_) => String::new(),
    }
}

/// Bootstrap tbug: create the home directory, then inject shell hooks.
pub fn init() -> Result<()> {
    let home = get_tbug_home();
    println!("tbug home directory: {}", home.display());

    let home_path = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()));

    inject_hook_to_file(&home_path.join(".zshrc"), ZSH_HOOK_SCRIPT)?;
    inject_hook_to_file(&home_path.join(".bashrc"), BASH_HOOK_SCRIPT)?;
    inject_hook_to_file(&home_path.join(".config/fish/config.fish"), FISH_HOOK_SCRIPT)?;

    println!("tbug initialized successfully.");
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inject_skips_missing_file() {
        let result = inject_hook_to_file(
            Path::new("/nonexistent/path/xyz_rc"),
            "echo test",
        );
        assert!(result.is_ok(), "missing file should silently succeed");
    }

    #[test]
    fn inject_adds_hook_to_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".testrc");
        fs::write(&rc, "").unwrap();

        inject_hook_to_file(&rc, "echo injected").unwrap();
        let content = fs::read_to_string(&rc).unwrap();

        assert!(content.contains("# === TBUG HOOK ==="));
        assert!(content.contains("echo injected"));
        assert!(content.contains("# === END TBUG HOOK ==="));
    }

    #[test]
    fn inject_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".testrc");
        fs::write(&rc, "export FOO=1\n").unwrap();

        // First injection.
        inject_hook_to_file(&rc, "hook_v1").unwrap();
        let after_first = fs::read_to_string(&rc).unwrap();

        // Second injection must not change the file.
        inject_hook_to_file(&rc, "hook_v2").unwrap();
        let after_second = fs::read_to_string(&rc).unwrap();

        assert_eq!(after_first, after_second);
        assert!(after_first.contains("hook_v1"));
        assert!(!after_first.contains("hook_v2"));
    }

    #[test]
    fn inject_preserves_existing_content() {
        let dir = tempfile::tempdir().unwrap();
        let rc = dir.path().join(".testrc");
        let original = "export PATH=/usr/bin:$PATH\nalias ll='ls -la'\n";
        fs::write(&rc, original).unwrap();

        inject_hook_to_file(&rc, "my_hook").unwrap();
        let content = fs::read_to_string(&rc).unwrap();

        assert!(content.starts_with(original));
        assert!(content.contains("# === TBUG HOOK ==="));
    }

    #[test]
    fn get_tbug_home_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let tbug_dir = dir.path().join(TBUG_DIR_NAME);
        // Override HOME so get_tbug_home points to our temp dir.
        std::env::set_var("HOME", dir.path());
        let path = get_tbug_home();
        assert_eq!(path, tbug_dir);
        assert!(path.exists());
    }
}
