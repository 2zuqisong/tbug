pub mod prompt;

use anyhow::Result;

use crate::client::{
    self, ChatMessage, ChatOptions, ChatStreamEvent, FunctionCall, ToolCall, ToolDefinition,
};
use crate::executor;
use crate::tools;
use crate::utils::ui;

use prompt::SYSTEM_PROMPT;

// ── Types ──────────────────────────────────────────────────────────

pub struct AgentOptions {
    /// The command to debug (e.g. `"cargo"`, `"npm"`).
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Maximum ReAct iterations before giving up.  Defaults to 10.
    pub max_iterations: usize,
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

// ── Agent loop ─────────────────────────────────────────────────────

/// Entry point: run the command, feed errors to the LLM, apply fixes,
/// re-run, and repeat until the command succeeds or iterations run out.
pub async fn run_agent(options: AgentOptions) -> Result<()> {
    let AgentOptions {
        command,
        args,
        max_iterations,
    } = options;

    let mut messages: Vec<ChatMessage> = vec![ChatMessage::system(SYSTEM_PROMPT)];

    // Capture CWD so the PTY child inherits the user's working directory.
    let working_dir = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from));

    print_separator(&format!("Running: {} {}", command, args.join(" ")));

    // ── Initial run ──────────────────────────────────────────────
    let mut pty_result = executor::run(
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
        println!("\n✓ Command succeeded. Nothing to debug.");
        return Ok(());
    }

    messages.push(ChatMessage::user(&build_user_message(
        &command,
        &args,
        &pty_result.output,
        0,
    )));

    // ── ReAct loop ───────────────────────────────────────────────
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

        if let Some(ref tcs) = response.tool_calls {
            assistant_msg.tool_calls = Some(
                tcs.iter()
                    .map(|tc| ToolCall {
                        id: tc.id.clone(),
                        call_type: "function".into(),
                        function: FunctionCall {
                            name: tc.name.clone(),
                            arguments: serde_json::to_string(&tc.arguments)
                                .unwrap_or_default(),
                        },
                    })
                    .collect(),
            );
        }
        messages.push(assistant_msg);

        // ── Execute tool calls ─────────────────────────────────
        if let Some(ref tcs) = response.tool_calls {
            if !tcs.is_empty() {
                for tc in tcs {
                    println!("\n  🔧 {}", tc.name);

                    // Edit-gate for destructive file writes
                    if tc.name == "patch_file" {
                        let confirmed = ui::ask_user_confirmation(&tc.arguments);
                        if !confirmed {
                            println!("  ⏭  Skipped (user declined).");
                            messages.push(ChatMessage::tool(
                                &tc.id,
                                "User declined this edit.",
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
        }

        // ── No tool calls — re-run command to verify ───────────
        print_separator(&format!("Re-running: {} {}", command, args.join(" ")));

        pty_result = executor::run(
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
            println!("\n✓ Command succeeded — bug fixed!");
            return Ok(());
        }

        messages.push(ChatMessage::user(&build_user_message(
            &command,
            &args,
            &pty_result.output,
            i + 1,
        )));
    }

    println!(
        "\n⚠ Max iterations ({}) reached. The issue may still be present.",
        max_iterations
    );
    println!(
        "  Review the changes and try running tbug again, or fix the remaining issues manually."
    );

    Ok(())
}
