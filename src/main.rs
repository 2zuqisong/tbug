mod agent;
mod client;
mod executor;
mod integration;
mod tools;
mod utils;

use std::process::{self, Command, Stdio};

use clap::{Parser, Subcommand};

/// tbug — AI-powered autonomous debugging assistant.
///
/// Feed it a failing command and tbug will diagnose the error,
/// apply fixes, and re-verify — fully autonomously.
#[derive(Parser)]
#[command(name = "tbug", version = "0.1.0")]
struct Cli {
    /// The command to debug (e.g. "cargo", "npm", "make").
    /// When omitted, tbug runs in error-diagnosis mode.
    command: Option<String>,

    /// Arguments passed to the command.
    #[arg(num_args = 0.., allow_hyphen_values = true, trailing_var_arg = true)]
    args: Vec<String>,

    /// Maximum ReAct iterations before giving up.
    #[arg(short = 'n', long, default_value = "10")]
    max_iterations: usize,

    #[command(subcommand)]
    subcommand: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize tbug configuration in the current environment.
    Init,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load .env, overriding shell env (matches TS precedence).
    client::load_env();

    // Subcommand dispatch
    if let Some(sub) = cli.subcommand {
        match sub {
            Commands::Init => {
                if let Err(e) = integration::init() {
                    eprintln!("init failed: {}", e);
                    process::exit(1);
                }
                return;
            }
        }
    }

    // No subcommand — dispatch by argument pattern
    match cli.command {
        Some(cmd) => {
            if cli.args.is_empty() {
                // Single-word command (e.g. `tbug make`) — direct agent mode
                if let Err(e) = agent::run_agent(agent::AgentOptions {
                    command: cmd,
                    args: vec![],
                    max_iterations: cli.max_iterations,
                })
                .await
                {
                    eprintln!("Fatal error: {}", e);
                    process::exit(1);
                }
            } else {
                // Multi-word input (e.g. `tbug 杀死 8080 端口`) — copilot mode
                let intent = format!("{} {}", cmd, cli.args.join(" "));
                let command = match agent::run_copilot(&intent).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Copilot error: {}", e);
                        process::exit(1);
                    }
                };

                // ── Safety gate ─────────────────────────────────
                if !confirm_execution(&command) {
                    println!("操作已取消");
                    process::exit(0);
                }

                // ── Native shell execution ──────────────────────
                let status = execute_shell(&command);
                process::exit(status.code().unwrap_or(1));
            }
        }
        None => {
            // Bare `tb` — environment diagnosis mode
            match integration::read_last_command() {
                Some(cmd) => {
                    let ctx = agent::DiagnosisContext {
                        last_cmd: cmd,
                        error_text: integration::read_last_error(),
                    };
                    if let Err(e) =
                        agent::run_diagnosis(ctx, cli.max_iterations).await
                    {
                        eprintln!("Fatal error: {}", e);
                        process::exit(1);
                    }
                }
                None => {
                    println!(
                        "当前无失败命令上下文。请使用 'tb <需求描述>' 开启 Copilot 模式。"
                    );
                    process::exit(0);
                }
            }
        }
    }
}

// ── Copilot helpers ──────────────────────────────────────────────────

/// Display the generated command and ask the user for execution
/// authorization via `dialoguer::Confirm`.  Returns `true` on `y`.
fn confirm_execution(command: &str) -> bool {
    println!();
    println!("🤖 [TBug] 智能体为您生成的系统命令如下：");
    println!("👉 {}", command);
    println!();
    println!("⚠️  警告：该命令将直接在您的宿主操作系统中运行！");
    println!();

    dialoguer::Confirm::new()
        .with_prompt("是否授权自动运行该命令?")
        .default(false)
        .interact()
        .unwrap_or(false)
}

/// Run `cmd` under the user's login shell (`$SHELL` or `sh` fallback)
/// with `-c`, inheriting stdout and stderr so native ANSI colors,
/// progress bars, and error messages pass through untouched.
fn execute_shell(cmd: &str) -> std::process::ExitStatus {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
    Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("Failed to execute command")
}
