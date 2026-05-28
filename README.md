```
 _____ ____
|_   _| __ ) _   _  __ _
  | | |  _ \| | | |/ _` |
  | | | |_) | |_| | (_| |
  |_| |____/ \__,_|\__, |
                   |___/  v0.1.0
```

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.80%2B-orange.svg)](https://www.rust-lang.org/)
[![LLM: DeepSeek V4](https://img.shields.io/badge/LLM-DeepSeek%20V4-blue.svg)](https://platform.deepseek.com)

tbug is an AI-powered autonomous debugging assistant. It diagnoses build
errors, test failures, and runtime crashes, applies fixes, and re-verifies
— fully autonomously.

---

## Quick Start

### 1. Clone and Build

```bash
git clone <repo-url> && cd tbug
cargo build --release
```

### 2. Configure Your API Key

Create a `.env` file in the project root (already gitignored):

```env
DEEPSEEK_API_KEY=sk-your-actual-api-key-here
```

### 3. Initialize Shell Hooks

```bash
tb init
```

Injects `preexec` / `precmd` hooks into `~/.zshrc`, `~/.bashrc`, and
`~/.config/fish/config.fish` so tbug can capture the last failed command
and its error output automatically.

---

## Usage Modes

### Diagnosis Mode (`tb`)

After a command fails in your shell, run `tb` with no arguments:

```bash
$ cargo build
error[E0308]: mismatched types ...

$ tb
```

tbug reads the last failed command from `~/.tbug/last_cmd.log` and the
error output from `~/.tbug/last_error.ansi`, then enters an autonomous
ReAct loop — inspecting files, applying patches, and re-running the
command until it passes.

### Copilot Mode (`tb <natural language>`)

Describe what you want in plain language and tbug translates it into a
shell command with a safety gate:

```bash
$ tb kill the process using port 8080

🤖 [TBug] Generated the following system command:
👉 fuser -k 8080/tcp

⚠️  WARNING: This command will run directly on your host OS!
Authorize automatic execution? (y/n)
```

Non-ASCII input (e.g. Chinese) is automatically detected and routed to
the copilot translator.

### Direct Debug Mode (`tb <command>`)

Feed tbug a single command name and it runs it, entering the ReAct loop
if it fails:

```bash
tb make
tb cargo
tb pytest
```

---

## Commands

| Command | Description |
| --- | --- |
| `tb` | Diagnose the last failed command captured by shell hooks |
| `tb <description>` | Translate natural language into a shell command |
| `tb <command>` | Run a command and auto-fix if it fails |
| `tb init` | Inject shell hooks into `.zshrc` / `.bashrc` / `config.fish` |
| `tb config` | Switch global interaction language (zh / en) |

---

## Configuration

### Environment Variables

| Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `DEEPSEEK_API_KEY` | **Yes** | — | DeepSeek platform API key |
| `DEEPSEEK_API_BASE` | No | `https://api.deepseek.com/v1` | Custom API endpoint or proxy |

> Configuration uses three-tier priority: `.env` file → shell environment
> → constructor parameter. `.env` values take highest precedence.

### App Config (`~/.tbug/config.json`)

```json
{
  "language": "zh"
}
```

Set `language` to `"zh"` (Simplified Chinese) or `"en"` (English).
Use `tb config` for an interactive menu, or edit the file directly.

---

## Architecture

```
main.rs  →  agent (ReAct loop)  →  executor (PTY)  →  LLM (client)  →  tools
  clap        prompt.rs              portable-pty       reqwest SSE       view/patch file
  config      Edit-Gate (ui)         std::thread        openai_types      diff engine
              I18n                   mpsc channels      OnceLock
```

- **ReAct loop** — Reason + Act: LLM streams thinking, requests tool
  calls, tbug executes them with an Edit-Gate safety check, then re-runs
  the original command via PTY.
- **Copilot** — Strict one-shot LLM call translates natural language to
  a raw shell command. Confirmation gate before execution via `$SHELL -c`.
- **Diff engine** — Three-tier SEARCH/REPLACE matching: exact → inject
  trailing `\n` → strip trailing `\n`. Uniqueness validation prevents
  ambiguous patches.
- **PTY executor** — `std::thread` + `mpsc` bridge between blocking PTY
  I/O and async runtime. Child handle in `Arc<Mutex<Option<Child>>>` for
  timeout `SIGKILL`.
- **Shell hooks** — ZSH `preexec`/`precmd`, Bash `PROMPT_COMMAND`, Fish
  `fish_preexec`/`fish_postexec` capture failed commands.

---

## Version

Current production version: **0.1.0**

## License

This project is open-sourced under the **[MIT License](https://opensource.org/licenses/MIT)**.
