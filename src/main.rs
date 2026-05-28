mod agent;
mod client;
mod executor;
mod integration;
mod tools;
mod utils;

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

pub fn build_diagnosis_prompt(last_cmd: &str, error_text: &str) -> String {
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
                    std::process::exit(1);
                }
                return;
            }
        }
    }

    // No subcommand — run the agent
    match cli.command {
        Some(cmd) => {
            if let Err(e) = agent::run_agent(agent::AgentOptions {
                command: cmd,
                args: cli.args,
                max_iterations: cli.max_iterations,
            })
            .await
            {
                eprintln!("Fatal error: {}", e);
                std::process::exit(1);
            }
        }
        None => {
            // Bare `tb` — environment diagnosis mode
            match integration::read_last_command() {
                Some(cmd) => {
                    let error_text = integration::read_last_error();
                    let prompt = build_diagnosis_prompt(&cmd, &error_text);
                    println!("{}", prompt);
                }
                None => {
                    println!(
                        "当前无失败命令上下文。请使用 'tb <需求描述>' 开启 Copilot 模式。"
                    );
                    std::process::exit(0);
                }
            }
        }
    }
}
