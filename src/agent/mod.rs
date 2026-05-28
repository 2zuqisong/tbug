pub mod prompt;

use anyhow::Result;

use crate::client::{
    self, ChatMessage, ChatOptions, ChatStreamEvent, FunctionCall, ToolCall, ToolDefinition,
};
use crate::config;
use crate::executor;
use crate::tools;
use crate::utils::ui;

use prompt::{COPILOT_SYSTEM_PROMPT, SYSTEM_PROMPT};

// ── Types ──────────────────────────────────────────────────────────

pub struct AgentOptions {
    /// The command to debug (e.g. `"cargo"`, `"npm"`).
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Maximum ReAct iterations before giving up.  Defaults to 10.
    pub max_iterations: usize,
    /// UI language: `"zh"` or `"en"`.
    pub language: String,
}

/// Context collected by the bare-`tb` diagnosis flow.
pub struct DiagnosisContext {
    /// Raw command line from `~/.tbug/last_cmd.log`.
    pub last_cmd: String,
    /// ANSI error capture from `~/.tbug/last_error.ansi` (may be empty).
    pub error_text: String,
    /// UI language: `"zh"` or `"en"`.
    pub language: String,
}

// ── Helpers ────────────────────────────────────────────────────────

fn build_user_message(command: &str, args: &[String], output: &str, iteration: usize) -> String {
    let full = if args.is_empty() {
        command.to_string()
    } else {
        format!("{} {}", command, args.join(" "))
    };
    if iteration == 0 {
        format!(
            "Please debug this failing command: `{}`\n\nError / output:\n```\n{}\n```",
            full, output
        )
    } else {
        format!(
            "The command `{}` is still failing after the previous fix:\n```\n{}\n```",
            full, output
        )
    }
}

/// Crude shell-parse: first word is the command, rest are args.
fn parse_command_line(line: &str) -> (String, Vec<String>) {
    let mut parts = line.split_whitespace();
    let cmd = parts.next().unwrap_or("").to_string();
    let args: Vec<String> = parts.map(|s| s.to_string()).collect();
    (cmd, args)
}

fn print_separator(label: &str) {
    let line = "─".repeat(50);
    println!("\n{}", line);
    println!("  {}", label);
    println!("{}\n", line);
}

/// Return cached tool definitions (JSON Schemas sent to the LLM).
fn get_tool_defs() -> Vec<ToolDefinition> {
    use std::sync::OnceLock;
    static CACHE: OnceLock<Vec<ToolDefinition>> = OnceLock::new();
    CACHE
        .get_or_init(tools::get_tool_definitions)
        .clone()
}

/// Serialise accumulated `ToolCallInfo` back to the OpenAI wire format
/// so the conversation history stays valid.
fn tool_info_to_tool_calls(
    tcs: &[crate::client::ToolCallInfo],
) -> Vec<ToolCall> {
    tcs.iter()
        .map(|tc| ToolCall {
            id: tc.id.clone(),
            call_type: "function".into(),
            function: FunctionCall {
                name: tc.name.clone(),
                arguments: serde_json::to_string(&tc.arguments).unwrap_or_default(),
            },
        })
        .collect()
}

// ── Shared ReAct core ──────────────────────────────────────────────

/// Inner loop shared by both `run_agent` and `run_diagnosis`.
async fn run_react_loop(
    mut messages: Vec<ChatMessage>,
    command: String,
    args: Vec<String>,
    working_dir: Option<String>,
    max_iterations: usize,
    language: &str,
) -> Result<()> {
    let cfg = config::AppConfig {
        language: language.to_string(),
    };

    for i in 0..max_iterations {
        print_separator(&format!("Agent iteration {}/{}", i + 1, max_iterations));

        let response = client::get_default_client()
            .chat_stream(
                &messages,
                Some(&ChatOptions {
                    tools: Some(get_tool_defs()),
                    ..Default::default()
                }),
                |event| match event {
                    ChatStreamEvent::Content { delta } => print!("{}", delta),
                    ChatStreamEvent::Thinking { delta } => print!("{}", delta),
                    ChatStreamEvent::Done => {}
                },
            )
            .await?;

        // ── Record assistant message ────────────────────────────
        let mut assistant_msg = ChatMessage {
            role: "assistant".into(),
            content: if response.content.is_empty() {
                None
            } else {
                Some(response.content.clone())
            },
            tool_calls: None,
            tool_call_id: None,
            name: None,
        };

        let has_tool_calls = response.tool_calls.as_ref()
            .map_or(false, |tcs| !tcs.is_empty());

        if has_tool_calls {
            let tcs = response.tool_calls.as_ref().unwrap();
            assistant_msg.tool_calls = Some(tool_info_to_tool_calls(tcs));
        }
        messages.push(assistant_msg);

        // ── Execute tool calls ─────────────────────────────────
        if has_tool_calls {
            let tcs = response.tool_calls.as_ref().unwrap();
            for tc in tcs {
                println!("\n  🔧 {}", tc.name);

                // Edit-Gate: confirm before destructive file writes
                if tc.name == "patch_file" {
                    let confirmed =
                        ui::ask_user_confirmation(&tc.arguments, language);
                    if !confirmed {
                        println!("  ⏭  {}", cfg.msg_user_declined());
                        messages.push(ChatMessage::tool(
                            &tc.id,
                            "User rejected this patch. Please try a different approach.",
                        ));
                        continue;
                    }
                }

                let result =
                    tools::execute_tool(&tc.name, &tc.arguments).await;
                let status = if result.success { "✓" } else { "✗" };
                let preview = if result.content.len() > 300 {
                    format!("{}...", &result.content[..300])
                } else {
                    result.content.clone()
                };
                println!(
                    "  {} {}",
                    status,
                    preview.split('\n').next().unwrap_or("")
                );

                messages.push(ChatMessage::tool(&tc.id, &result.content));
            }
            continue; // back to LLM with tool results
        }

        // ── No tool calls — re-run command to verify ───────────
        print_separator(&format!("Re-running: {} {}", command, args.join(" ")));

        let pty_result = executor::run(
            executor::PtyOptions {
                command: command.clone(),
                args: args.clone(),
                cwd: working_dir.clone(),
                env: None,
                timeout: None,
            },
            |data| print!("{}", data),
        )
        .await?;

        if pty_result.exit_code == 0 {
            println!("\n🎉 {}", cfg.msg_bug_fixed());
            return Ok(());
        }

        messages.push(ChatMessage::user(&build_user_message(
            &command,
            &args,
            &pty_result.output,
            i + 1,
        )));
    }

    println!("\n⚠ {}", cfg.msg_max_iterations(max_iterations));

    Ok(())
}

// ── Public entry points ────────────────────────────────────────────

/// Entry point for `tbug <command> [args...]`.
pub async fn run_agent(options: AgentOptions) -> Result<()> {
    let cfg = config::AppConfig {
        language: options.language.clone(),
    };
    let language = options.language.clone();

    let AgentOptions {
        command,
        args,
        max_iterations,
        ..
    } = options;

    let working_dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from));

    print_separator(&format!("Running: {} {}", command, args.join(" ")));

    // ── Initial run ──────────────────────────────────────────────
    let pty_result = executor::run(
        executor::PtyOptions {
            command: command.clone(),
            args: args.clone(),
            cwd: working_dir.clone(),
            env: None,
            timeout: None,
        },
        |data| print!("{}", data),
    )
    .await?;

    if pty_result.exit_code == 0 {
        println!("\n✓ {}", cfg.msg_nothing_to_debug());
        return Ok(());
    }

    let system_msg = format!("{}{}", SYSTEM_PROMPT, cfg.language_constraint());

    let messages = vec![
        ChatMessage::system(&system_msg),
        ChatMessage::user(&build_user_message(&command, &args, &pty_result.output, 0)),
    ];

    run_react_loop(messages, command, args, working_dir, max_iterations, &language).await
}

/// Entry point for bare `tb` (diagnosis mode).
pub async fn run_diagnosis(ctx: DiagnosisContext, max_iterations: usize) -> Result<()> {
    let cfg = config::AppConfig {
        language: ctx.language.clone(),
    };
    let language = ctx.language.clone();

    let (command, args) = parse_command_line(&ctx.last_cmd);

    let working_dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from));

    print_separator(&format!(
        "Diagnosis: {} {}",
        command,
        args.join(" ")
    ));

    let prompt = cfg.diagnosis_prompt(&ctx.last_cmd, &ctx.error_text);
    let system_msg = format!("{}{}", SYSTEM_PROMPT, cfg.language_constraint());

    let messages = vec![
        ChatMessage::system(&system_msg),
        ChatMessage::user(&prompt),
    ];

    run_react_loop(messages, command, args, working_dir, max_iterations, &language).await
}

/// Copilot mode: translate a natural-language intent into a clean shell
/// command using a strict one-shot LLM call.
pub async fn run_copilot(intent: &str, language: &str) -> Result<String> {
    let cfg = config::AppConfig {
        language: language.to_string(),
    };

    // Append language constraint to the copilot system prompt.
    let copilot_prompt = if language == "en" {
        format!(
            "{}\n\nADDITIONAL RULE: If you must include any explanatory text \
             (which you should not), it MUST be in English.",
            COPILOT_SYSTEM_PROMPT
        )
    } else {
        COPILOT_SYSTEM_PROMPT.to_string()
    };

    let messages = vec![
        ChatMessage::system(&copilot_prompt),
        ChatMessage::user(intent),
    ];

    let _ = cfg; // used if we add more language-specific behaviour

    let response = client::get_default_client()
        .chat_stream(
            &messages,
            None,
            |_| {}, // silent — only interested in the final content
        )
        .await?;

    let raw = response.content.trim().to_string();

    // Strip common markdown fence leftovers in case the model disobeyed.
    let cleaned = raw
        .strip_prefix("```bash")
        .or_else(|| raw.strip_prefix("```sh"))
        .or_else(|| raw.strip_prefix("```shell"))
        .or_else(|| raw.strip_prefix("```"))
        .unwrap_or(&raw)
        .strip_suffix("```")
        .unwrap_or(&raw)
        .trim()
        .to_string();

    Ok(cleaned)
}
