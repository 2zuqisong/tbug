use std::collections::HashMap;
use std::io::Read;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use tokio::sync::mpsc;

// ── Types ──────────────────────────────────────────────────────────

/// Configuration for a PTY command execution.
#[derive(Debug, Clone)]
pub struct PtyOptions {
    /// The executable to run (e.g. `"cargo"`, `"npm"`).
    pub command: String,
    /// Arguments passed to the command.
    pub args: Vec<String>,
    /// Working directory for the child process.
    pub cwd: Option<String>,
    /// Extra environment variables merged on top of `process.env`.
    pub env: Option<HashMap<String, String>>,
    /// If set, the child receives SIGKILL after this duration and `exit_code`
    /// is reported as `-1`.
    pub timeout: Option<Duration>,
}

/// The outcome of a PTY execution.
#[derive(Debug, Clone)]
pub struct PtyResult {
    /// Merged stdout + stderr captured from the pseudoterminal.
    pub output: String,
    /// Process exit code. `-1` when the process was killed by timeout.
    pub exit_code: i32,
    /// Signal number if the process was terminated by a signal.
    pub signal: Option<i32>,
}

// ── Public API ─────────────────────────────────────────────────────

/// Execute `command` under a pseudoterminal, forwarding output in real time
/// via `on_data`, and return the aggregated result.
///
/// # Architecture
///
/// A `std::thread` owns the blocking PTY read loop and sends data chunks to
/// the async context through a `tokio::sync::mpsc::unbounded_channel`.
/// The child process handle is kept in an `Arc<Mutex<..>>` so the async
/// timeout path can directly issue `SIGKILL`, which causes the blocking
/// `read()` in the thread to unblock with EOF.
///
/// ```
///  std::thread                      async context (tokio)
///  ───────────                      ─────────────────────
///  spawn child                      ┌─ mpsc::recv() loop
///  read() → mpsc::send() ──────────→│  on_data(chunk)
///  read() → mpsc::send()            │  on_data(chunk)
///  ...                              │  ...
///  EOF → drop(mpsc::Sender) ───────→│  recv() returns None
///  write result → Mutex             │  read result ← Mutex
///                                   │
///  [timeout]                        │  lock Mutex → child.kill()
///     ↑ SIGKILL sent from here ─────────┘
/// ```
pub async fn run(
    options: PtyOptions,
    mut on_data: impl FnMut(&str),
) -> Result<PtyResult> {
    let PtyOptions {
        command,
        args,
        cwd,
        env,
        timeout,
    } = options;

    // ── Shared state between the blocking thread and async context ─
    let (data_tx, mut data_rx) = mpsc::unbounded_channel::<String>();
    // Child handle so the async timeout path can kill the process.
    let child_holder: Arc<Mutex<Option<Box<dyn portable_pty::Child + Send>>>> =
        Arc::new(Mutex::new(None));
    // Final result written by the thread before dropping data_tx.
    let result_holder: Arc<Mutex<Option<PtyResult>>> = Arc::new(Mutex::new(None));

    let child_holder_th = Arc::clone(&child_holder);
    let result_holder_th = Arc::clone(&result_holder);

    // ── Spawn blocking PTY thread ────────────────────────────────
    let _thread = std::thread::spawn(move || {
        run_pty_blocking(
            &command,
            &args,
            cwd.as_deref(),
            env.as_ref(),
            data_tx,
            child_holder_th,
            result_holder_th,
        );
    });

    // ── Forward streamed data; honour optional timeout ───────────
    let mut timed_out = false;

    loop {
        if timed_out {
            // After SIGKILL the thread will soon finish — just drain remaining data.
            match data_rx.recv().await {
                Some(chunk) => on_data(&chunk),
                None => break,
            }
        } else if let Some(dur) = timeout {
            tokio::select! {
                chunk = data_rx.recv() => {
                    match chunk {
                        Some(s) => on_data(&s),
                        None => break,
                    }
                }
                _ = tokio::time::sleep(dur) => {
                    timed_out = true;
                    // Reach into the thread and kill the child.
                    if let Some(mut child) = child_holder.lock().unwrap().take() {
                        let _ = child.kill();
                    }
                }
            }
        } else {
            match data_rx.recv().await {
                Some(chunk) => on_data(&chunk),
                None => break,
            }
        }
    }

    // ── Collect result ───────────────────────────────────────────
    let mut result = result_holder
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| PtyResult {
            output: String::new(),
            exit_code: -1,
            signal: None,
        });

    if timed_out {
        result.exit_code = -1;
    }

    Ok(result)
}

// ── Blocking PTY logic (runs on std::thread) ───────────────────────

fn run_pty_blocking(
    command: &str,
    args: &[String],
    cwd: Option<&str>,
    env: Option<&HashMap<String, String>>,
    data_tx: mpsc::UnboundedSender<String>,
    child_holder: Arc<Mutex<Option<Box<dyn portable_pty::Child + Send>>>>,
    result_holder: Arc<Mutex<Option<PtyResult>>>,
) {
    let mut finish = |r: PtyResult| {
        *result_holder.lock().unwrap() = Some(r);
    };

    // ── Open PTY ───────────────────────────────────────────────
    let pty_system = native_pty_system();
    let pair = match pty_system.openpty(PtySize::default()) {
        Ok(p) => p,
        Err(e) => {
            return finish(PtyResult {
                output: format!("Failed to open PTY: {}", e),
                exit_code: -1,
                signal: None,
            });
        }
    };

    // ── Build command ──────────────────────────────────────────
    let mut cmd = CommandBuilder::new(command);
    cmd.args(args);
    if let Some(cwd) = cwd {
        cmd.cwd(cwd);
    }
    if let Some(env) = env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }

    // ── Spawn child ────────────────────────────────────────────
    let child = match pair.slave.spawn_command(cmd) {
        Ok(c) => c,
        Err(e) => {
            return finish(PtyResult {
                output: format!("Failed to spawn \"{}\": {}", command, e),
                exit_code: -1,
                signal: None,
            });
        }
    };

    // Store child handle for async timeout path.
    *child_holder.lock().unwrap() = Some(child);

    // Drop slave so the master can read.
    drop(pair.slave);

    let mut reader = match pair.master.try_clone_reader() {
        Ok(r) => r,
        Err(e) => {
            return finish(PtyResult {
                output: format!("Failed to clone master reader: {}", e),
                exit_code: -1,
                signal: None,
            });
        }
    };

    // ── Read loop ──────────────────────────────────────────────
    let mut output = String::new();
    let mut buf = [0u8; 4096];

    loop {
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                output.push_str(&chunk);
                if data_tx.send(chunk).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    // ── Wait for child ─────────────────────────────────────────
    let mut child_opt = child_holder.lock().unwrap().take();
    let (exit_code, signal) = match child_opt.as_mut() {
        Some(child) => match child.wait() {
            Ok(status) => (status.exit_code() as i32, None),
            Err(_) => (0, None),
        },
        None => {
            (-1, None)
        }
    };

    finish(PtyResult {
        output,
        exit_code,
        signal,
    })
}

// ── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Helper: run a command and collect all output.
    async fn collect(cmd: &str, args: &[&str], timeout_ms: Option<u64>) -> PtyResult {
        let opts = PtyOptions {
            command: cmd.to_string(),
            args: args.iter().map(|s| s.to_string()).collect(),
            cwd: None,
            env: None,
            timeout: timeout_ms.map(Duration::from_millis),
        };
        let mut events: Vec<String> = Vec::new();
        let result = run(opts, |s| events.push(s.to_string())).await.unwrap();
        // Sanity: events were forwarded
        let combined: String = events.concat();
        assert!(!combined.is_empty() || result.exit_code != 0 || result.output.is_empty());
        result
    }

    // ── Normal exit (exit code 0) ──────────────────────────────

    #[tokio::test]
    async fn normal_exit_echo() {
        let result = collect("echo", &["hello world"], None).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.output.contains("hello world"));
    }

    #[tokio::test]
    async fn normal_exit_multiline() {
        let result = collect("sh", &["-c", "echo line1 && echo line2"], None).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.output.contains("line1"));
        assert!(result.output.contains("line2"));
    }

    #[tokio::test]
    async fn normal_exit_large_output() {
        // Generate enough output to span multiple 4 KiB reads
        let result = collect(
            "sh",
            &["-c", "for i in $(seq 1 200); do echo \"line $i\"; done"],
            None,
        )
        .await;
        assert_eq!(result.exit_code, 0);
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 200"));
    }

    // ── Non-zero exit code ─────────────────────────────────────

    #[tokio::test]
    async fn non_zero_exit_code() {
        let result = collect("sh", &["-c", "echo 'failed!' >&2; exit 3"], None).await;
        assert_eq!(result.exit_code, 3);
        // PTY merges stdout + stderr
        assert!(result.output.contains("failed!"));
    }

    #[tokio::test]
    async fn non_zero_exit_code_command_not_found() {
        let result = collect("__nonexistent_command_xyz__", &[], None).await;
        assert_ne!(result.exit_code, 0);
    }

    #[tokio::test]
    async fn non_zero_exit_with_partial_output() {
        let result = collect(
            "sh",
            &["-c", "echo 'before crash'; exit 7; echo 'after crash'"],
            None,
        )
        .await;
        assert_eq!(result.exit_code, 7);
        assert!(result.output.contains("before crash"));
        assert!(!result.output.contains("after crash"));
    }

    // ── Timeout with SIGKILL ───────────────────────────────────

    #[tokio::test]
    async fn timeout_kills_sleeping_process() {
        let result = collect("sleep", &["10"], Some(100)).await;
        assert_eq!(result.exit_code, -1, "exit_code should be -1 on timeout");
    }

    #[tokio::test]
    async fn timeout_captures_partial_output() {
        // Command prints something quickly, then stalls forever
        let result = collect(
            "sh",
            &["-c", "echo 'started'; sleep 10"],
            Some(200),
        )
        .await;
        assert_eq!(result.exit_code, -1);
        assert!(
            result.output.contains("started"),
            "partial output before timeout should be captured"
        );
    }

    #[tokio::test]
    async fn no_timeout_completes_normally() {
        // Fast command with a generous timeout — should NOT time out
        let result = collect("echo", &["done"], Some(5000)).await;
        assert_eq!(result.exit_code, 0);
        assert!(result.output.contains("done"));
    }

    // ── on_data callback ───────────────────────────────────────

    #[tokio::test]
    async fn on_data_receives_chunks_in_order() {
        let opts = PtyOptions {
            command: "echo".into(),
            args: vec!["chunk1".into()],
            cwd: None,
            env: None,
            timeout: None,
        };
        let mut received: Vec<String> = Vec::new();
        run(opts, |s| received.push(s.to_string()))
            .await
            .unwrap();
        assert!(!received.is_empty(), "on_data should be called at least once");
        let combined: String = received.concat();
        assert!(combined.contains("chunk1"));
    }
}
