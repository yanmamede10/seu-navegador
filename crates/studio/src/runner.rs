//! Integrated bounded-memory command runner / terminal sessions for Studio.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

const MAX_OUTPUT_BYTES: usize = 128 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSnapshot {
    pub session_id: u64,
    pub command: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub output: String,
}

struct SessionInner {
    session_id: u64,
    command: String,
    running: bool,
    exit_code: Option<i32>,
    output: String,
    child: Option<Child>,
}

pub type TerminalNotifyFn = Arc<dyn Fn(TerminalSnapshot) + Send + Sync>;

#[derive(Default)]
pub struct TerminalManager {
    next_id: u64,
    sessions: HashMap<u64, Arc<Mutex<SessionInner>>>,
}

impl TerminalManager {
    pub fn spawn_command(
        &mut self,
        cwd: &Path,
        command: &str,
        on_update: Option<TerminalNotifyFn>,
    ) -> Result<TerminalSnapshot, String> {
        let trimmed = command.trim();
        if trimmed.is_empty() {
            return Err("command cannot be empty".to_string());
        }

        self.next_id += 1;
        let session_id = self.next_id;

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = Command::new("powershell.exe");
            c.args(["-NoProfile", "-NonInteractive", "-Command", trimmed]);
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let mut c = Command::new("sh");
            c.args(["-c", trimmed]);
            c
        };

        let mut child = cmd
            .current_dir(cwd)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("failed to start command: {e}"))?;

        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let inner = Arc::new(Mutex::new(SessionInner {
            session_id,
            command: trimmed.to_string(),
            running: true,
            exit_code: None,
            output: format!("$ {trimmed}\n"),
            child: Some(child),
        }));
        self.sessions.insert(session_id, inner.clone());

        if let Some(out) = stdout {
            let inner_clone = inner.clone();
            let notify = on_update.clone();
            let _ = thread::Builder::new()
                .name(format!("seu-term-out-{session_id}"))
                .spawn(move || pump_stream(out, inner_clone, notify));
        }

        if let Some(err) = stderr {
            let inner_clone = inner.clone();
            let notify = on_update.clone();
            let _ = thread::Builder::new()
                .name(format!("seu-term-err-{session_id}"))
                .spawn(move || pump_stream(err, inner_clone, notify));
        }

        // Watcher thread waits for child completion and publishes final exit code.
        let inner_wait = inner.clone();
        let notify_wait = on_update;
        let _ = thread::Builder::new()
            .name(format!("seu-term-wait-{session_id}"))
            .spawn(move || loop {
                let done = {
                    let Ok(mut guard) = inner_wait.lock() else {
                        return;
                    };
                    if let Some(ref mut ch) = guard.child {
                        match ch.try_wait() {
                            Ok(Some(status)) => {
                                guard.running = false;
                                guard.exit_code = status.code();
                                guard.child = None;
                                Some(snapshot_from(&guard))
                            }
                            Ok(None) => None,
                            Err(_) => {
                                guard.running = false;
                                guard.child = None;
                                Some(snapshot_from(&guard))
                            }
                        }
                    } else {
                        guard.running = false;
                        Some(snapshot_from(&guard))
                    }
                };
                if let Some(snap) = done {
                    if let Some(cb) = &notify_wait {
                        cb(snap);
                    }
                    break;
                }
                thread::sleep(std::time::Duration::from_millis(80));
            });

        let snap = inner
            .lock()
            .map(|g| snapshot_from(&g))
            .unwrap_or(TerminalSnapshot {
                session_id,
                command: trimmed.to_string(),
                running: true,
                exit_code: None,
                output: format!("$ {trimmed}\n"),
            });
        Ok(snap)
    }

    pub fn kill_session(&mut self, session_id: u64) -> Option<TerminalSnapshot> {
        let entry = self.sessions.get(&session_id)?.clone();
        let mut guard = entry.lock().ok()?;
        if let Some(ref mut child) = guard.child {
            let _ = child.kill();
        }
        guard.child = None;
        guard.running = false;
        append_bounded(&mut guard.output, "\n[process terminated]\n");
        Some(snapshot_from(&guard))
    }

    pub fn get_snapshot(&self, session_id: u64) -> Option<TerminalSnapshot> {
        let entry = self.sessions.get(&session_id)?;
        let guard = entry.lock().ok()?;
        Some(snapshot_from(&guard))
    }

    pub fn kill_all(&mut self) {
        for entry in self.sessions.values() {
            if let Ok(mut guard) = entry.lock() {
                if let Some(ref mut child) = guard.child {
                    let _ = child.kill();
                }
                guard.child = None;
                guard.running = false;
            }
        }
        self.sessions.clear();
    }
}

fn pump_stream<R: std::io::Read>(
    stream: R,
    inner: Arc<Mutex<SessionInner>>,
    notify: Option<TerminalNotifyFn>,
) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(text) = line else { break };
        let snap = {
            let Ok(mut guard) = inner.lock() else {
                return;
            };
            append_bounded(&mut guard.output, &text);
            append_bounded(&mut guard.output, "\n");
            snapshot_from(&guard)
        };
        if let Some(cb) = &notify {
            cb(snap);
        }
    }
}

fn append_bounded(buf: &mut String, chunk: &str) {
    buf.push_str(chunk);
    if buf.len() > MAX_OUTPUT_BYTES {
        let excess = buf.len() - MAX_OUTPUT_BYTES;
        let mut cut = excess;
        while cut < buf.len() && !buf.is_char_boundary(cut) {
            cut += 1;
        }
        buf.drain(..cut);
    }
}

fn snapshot_from(inner: &SessionInner) -> TerminalSnapshot {
    TerminalSnapshot {
        session_id: inner.session_id,
        command: inner.command.clone(),
        running: inner.running,
        exit_code: inner.exit_code,
        output: inner.output.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runs_command_and_captures_output() {
        let mut mgr = TerminalManager::default();
        let cwd = std::env::temp_dir();
        let started = mgr.spawn_command(&cwd, "echo studio_ok", None);
        assert!(started.is_ok());
        if let Ok(snap) = started {
            for _ in 0..40 {
                if let Some(cur) = mgr.get_snapshot(snap.session_id) {
                    if !cur.running {
                        assert!(cur.output.contains("studio_ok"));
                        assert_eq!(cur.exit_code, Some(0));
                        return;
                    }
                }
                thread::sleep(std::time::Duration::from_millis(50));
            }
        }
    }
}
