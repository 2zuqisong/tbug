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

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load .env, overriding shell env (matches TS precedence).
    client::load_env();

    // Subcommand dispatch
    if let Some(sub) = cli.subcommand {
        match sub {
            Commands::Init => {
                integration::init();
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
            eprintln!("Error diagnosis mode is not yet implemented.");
            eprintln!("Usage: tbug <command> [args...]");
            eprintln!("       tbug init");
            std::process::exit(1);
        }
    }
}
