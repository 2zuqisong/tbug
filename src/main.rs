mod agent;
mod client;
mod config;
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

    /// Override the default LLM model (e.g. "deepseek-chat", "deepseek-reasoner").
    #[arg(short = 'm', long)]
    model: Option<String>,

    #[command(subcommand)]
    subcommand: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize tbug shell hooks and configuration.
    Init,
    /// Switch global interaction language (zh / en).
    Config,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load .env, overriding shell env (matches TS precedence).
    client::load_env();

    // Load persisted config (defaults to language=zh).
    let cfg = config::AppConfig::load();

    // Subcommand dispatch
    if let Some(sub) = cli.subcommand {
        match sub {
            Commands::Init => {
                if let Err(e) = integration::init(&cfg) {
                    eprintln!("init failed: {}", e);
                    process::exit(1);
                }
                return;
            }
            Commands::Config => {
                run_config(&cfg);
                return;
            }
        }
    }

    // No subcommand — dispatch by argument pattern
    match cli.command {
        Some(cmd) => {
            if cli.args.is_empty() && !is_natural_language(&cmd) {
                // Single ASCII word (e.g. `tbug make`) — direct agent mode
                if let Err(e) = agent::run_agent(agent::AgentOptions {
                    command: cmd,
                    args: vec![],
                    max_iterations: cli.max_iterations,
                    language: cfg.language.clone(),
                    model: cli.model.clone(),
                })
                .await
                {
                    eprintln!("Fatal error: {}", e);
                    process::exit(1);
                }
            } else {
                // Multi-word OR non-ASCII — copilot mode
                let intent = if cli.args.is_empty() {
                    cmd
                } else {
                    format!("{} {}", cmd, cli.args.join(" "))
                };
                let command = match agent::run_copilot(&intent, &cfg.language, cli.model.as_deref()).await {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("Copilot error: {}", e);
                        process::exit(1);
                    }
                };

                if !confirm_execution(&command, &cfg) {
                    println!("{}", cfg.msg_cancelled());
                    process::exit(0);
                }
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
                        language: cfg.language.clone(),
                        model: cli.model.clone(),
                    };
                    if let Err(e) =
                        agent::run_diagnosis(ctx, cli.max_iterations).await
                    {
                        eprintln!("Fatal error: {}", e);
                        process::exit(1);
                    }
                }
                None => {
                    println!("{}", cfg.msg_no_context());
                    process::exit(0);
                }
            }
        }
    }
}

// ── Subcommand handlers ────────────────────────────────────────────

fn run_config(cfg: &config::AppConfig) {
    let default_idx = if cfg.language == "en" { 1 } else { 0 };
    let selection = dialoguer::Select::new()
        .with_prompt(cfg.msg_select_language())
        .items(config::AppConfig::language_options())
        .default(default_idx)
        .interact()
        .unwrap_or(default_idx);

    let new_lang = if selection == 1 { "en" } else { "zh" };
    if new_lang == cfg.language {
        println!("✔ {}", cfg.msg_config_updated());
        return;
    }

    let mut updated = cfg.clone();
    updated.language = new_lang.to_string();

    if let Err(e) = updated.save() {
        eprintln!("Failed to save config: {}", e);
        process::exit(1);
    }

    println!("✔ {}", updated.msg_config_updated());
}

// ── Copilot helpers ──────────────────────────────────────────────────

fn is_natural_language(s: &str) -> bool {
    s.chars().any(|c| !c.is_ascii())
}

fn confirm_execution(command: &str, cfg: &config::AppConfig) -> bool {
    println!();
    println!("🤖 {}", cfg.msg_copilot_header());
    println!("👉 {}", command);
    println!();
    println!("⚠️  {}", cfg.msg_copilot_warning());
    println!();

    dialoguer::Confirm::new()
        .with_prompt(cfg.msg_copilot_authorize())
        .default(false)
        .interact()
        .unwrap_or(false)
}

fn execute_shell(cmd: &str) -> std::process::ExitStatus {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "sh".to_string());
    match Command::new(&shell)
        .arg("-c")
        .arg(cmd)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
    {
        Ok(status) => status,
        Err(e) => {
            eprintln!("Failed to execute command via {}: {}", shell, e);
            process::exit(1);
        }
    }
}
