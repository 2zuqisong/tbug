use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::integration;

// ── Config entity ───────────────────────────────────────────────────

fn default_language() -> String {
    "zh".to_string()
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct AppConfig {
    #[serde(default = "default_language")]
    pub language: String,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            language: default_language(),
        }
    }
}

// ── Persistence ─────────────────────────────────────────────────────

impl AppConfig {
    fn path() -> PathBuf {
        integration::get_tbug_home().join("config.json")
    }

    /// Load config from `~/.tbug/config.json`.  Falls back to defaults
    /// when the file is missing or malformed.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Persist config as pretty-printed JSON to `~/.tbug/config.json`.
    pub fn save(&self) -> Result<()> {
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize config")?;
        std::fs::write(Self::path(), json)
            .context("Failed to write config.json")?;
        Ok(())
    }
}

// ── I18n helpers ────────────────────────────────────────────────────

impl AppConfig {
    /// "No failed command context" — bare `tb` fallback.
    pub fn msg_no_context(&self) -> &'static str {
        if self.language == "en" {
            "No failed command context found. Use 'tb <description>' to enter Copilot mode."
        } else {
            "当前无失败命令上下文。请使用 'tb <需求描述>' 开启 Copilot 模式。"
        }
    }

    /// Copilot safety-gate: header line.
    pub fn msg_copilot_header(&self) -> &'static str {
        if self.language == "en" {
            "[TBug] Generated the following system command:"
        } else {
            "[TBug] 智能体为您生成的系统命令如下："
        }
    }

    /// Copilot safety-gate: warning.
    pub fn msg_copilot_warning(&self) -> &'static str {
        if self.language == "en" {
            "WARNING: This command will run directly on your host operating system!"
        } else {
            "警告：该命令将直接在您的宿主操作系统中运行！"
        }
    }

    /// Copilot safety-gate: y/n prompt.
    pub fn msg_copilot_authorize(&self) -> &'static str {
        if self.language == "en" {
            "Authorize automatic execution?"
        } else {
            "是否授权自动运行该命令?"
        }
    }

    /// Copilot: user cancelled.
    pub fn msg_cancelled(&self) -> &'static str {
        if self.language == "en" {
            "Operation cancelled."
        } else {
            "操作已取消"
        }
    }

    /// tb init: success banner.
    pub fn msg_init_ok(&self) -> &'static str {
        if self.language == "en" {
            "Fish/Zsh hook installed successfully."
        } else {
            "Fish/Zsh 钩子注入成功。"
        }
    }

    /// tb init: home dir banner.
    pub fn msg_init_home(&self) -> &'static str {
        if self.language == "en" {
            "tbug home directory:"
        } else {
            "tbug home directory:"
        }
    }

    /// ReAct: bug fixed.
    pub fn msg_bug_fixed(&self) -> &'static str {
        if self.language == "en" {
            "Build/run passed! Bug successfully defeated."
        } else {
            "编译/运行通过！Bug 已成功降伏。"
        }
    }

    /// ReAct: command succeeded (nothing to debug).
    pub fn msg_nothing_to_debug(&self) -> &'static str {
        if self.language == "en" {
            "Command succeeded. Nothing to debug."
        } else {
            "Command succeeded. Nothing to debug."
        }
    }

    /// ReAct: max iterations reached.
    pub fn msg_max_iterations(&self, n: usize) -> String {
        if self.language == "en" {
            format!(
                "Max iterations ({}) reached. The issue may still be present.\n  Review the changes and try running tbug again, or fix the remaining issues manually.",
                n
            )
        } else {
            format!(
                "已达最大迭代次数 ({})。问题可能仍然存在。\n  请检查更改并重试，或手动修复剩余问题。",
                n
            )
        }
    }

    /// ReAct: user declined Edit-Gate.
    pub fn msg_user_declined(&self) -> &'static str {
        if self.language == "en" {
            "Skipped (user declined)."
        } else {
            "已跳过（用户拒绝）。"
        }
    }

    /// Diagnosis prompt template (for run_diagnosis).
    pub fn diagnosis_prompt(&self, last_cmd: &str, error_text: &str) -> String {
        if self.language == "en" {
            format!(
                "The user just ran: `{}`\n\
                 The command crashed with the following terminal error:\n\
                 ```text\n\
                 {}\n\
                 ```\n\
                 Please analyse the scene using your toolchain and attempt a fix.",
                last_cmd, error_text
            )
        } else {
            format!(
                "用户刚刚运行了命令：`{}`\n\
                 该命令崩溃并抛出了以下终端错误树：\n\
                 ```text\n\
                 {}\n\
                 ```\n\
                 请利用你的工具链分析该现场，并尝试修复。",
                last_cmd, error_text
            )
        }
    }

    /// Language constraint appended to the system prompt in diagnosis mode.
    pub fn language_constraint(&self) -> &'static str {
        if self.language == "en" {
            "\n\nIMPORTANT: You MUST perform all technical analysis, reasoning, \
             and explanations strictly in English. Your tool call results and \
             final answers must be in English."
        } else {
            ""
        }
    }

    /// `tb config` Select prompt.
    pub fn msg_select_language(&self) -> &'static str {
        "请选择 tb 的全局交互语言 / Select global language for tb:"
    }

    /// `tb config` success toast.
    pub fn msg_config_updated(&self) -> &'static str {
        if self.language == "en" {
            "Configuration updated!"
        } else {
            "配置已更新！"
        }
    }

    /// Language labels for the Select menu.
    pub fn language_options() -> &'static [&'static str] {
        &["简体中文 (zh)", "English (en)"]
    }
}
