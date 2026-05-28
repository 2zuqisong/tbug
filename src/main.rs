mod agent;
mod client;
mod executor;
mod tools;
mod utils;

use clap::Parser;

/// tbug — AI-powered autonomous debugging assistant.
///
/// Feed it a failing command and tbug will diagnose the error,
/// apply fixes, and re-verify — fully autonomously.
#[derive(Parser)]
#[command(name = "tbug", version = "0.1.0")]
struct Cli {
    /// The command to debug (e.g. "cargo", "npm", "make").
    command: String,

    /// Arguments passed to the command.
    #[arg(num_args = 0.., allow_hyphen_values = true, trailing_var_arg = true)]
    args: Vec<String>,

    /// Maximum ReAct iterations before giving up.
    #[arg(short = 'n', long, default_value = "10")]
    max_iterations: usize,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    // Load .env, overriding shell env (matches TS precedence).
    client::load_env();

    if let Err(e) = agent::run_agent(agent::AgentOptions {
        command: cli.command,
        args: cli.args,
        max_iterations: cli.max_iterations,
    })
    .await
    {
        eprintln!("Fatal error: {}", e);
        std::process::exit(1);
    }
}
