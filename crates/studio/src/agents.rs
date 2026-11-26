//! Hybrid Multi-Agent Orchestrator supporting both direct LLM HTTP APIs
//! (Gemini, Anthropic Claude, OpenAI, local Ollama) and installed local CLIs
//! (`gemini`, `claude`, `codex`, or custom CLI command).
//!
//! All file modifications proposed by agents are parsed into `ProposedPatch`
//! objects and require explicit user approval before writing to disk.

use crate::workspace::{list_directory, read_file_text, write_file_text, WorkspaceError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentRole {
    Planner,
    Coder,
    QaBrowser,
}

impl AgentRole {
    pub fn label(self) -> &'static str {
        match self {
            Self::Planner => "Planner",
            Self::Coder => "Coder",
            Self::QaBrowser => "QA Browser",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderKind {
    #[default]
    GeminiApi,
    AnthropicApi,
    OpenAiApi,
    OllamaLocal,
    CliGemini,
    CliClaude,
    CliCodex,
    CliCustom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProviderConfig {
    pub provider: ProviderKind,
    pub model: String,
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default)]
    pub custom_cli_cmd: String,
}

impl Default for AgentProviderConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::OllamaLocal,
            model: "qwen2.5-coder:7b".to_string(),
            api_key: String::new(),
            base_url: "http://127.0.0.1:11434".to_string(),
            custom_cli_cmd: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DetectedLocalCli {
    pub id: String,
    pub label: String,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct QaConsoleEntry {
    pub level: String,
    pub message: String,
    pub source: String,
    pub line: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BrowserQaSnapshot {
    pub tab_id: u64,
    pub url: String,
    pub title: String,
    pub viewport_w: i32,
    pub viewport_h: i32,
    pub interactive_count: usize,
    pub interactive_summary: Vec<String>,
    pub text_excerpt: String,
    pub console_errors: Vec<QaConsoleEntry>,
    pub console_warnings: Vec<QaConsoleEntry>,
    pub captured_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QaActionResult {
    pub ok: bool,
    pub action: String,
    pub selector: String,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedPatch {
    pub rel_path: String,
    pub new_content: String,
    pub preview: String,
    pub applied: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRunSnapshot {
    pub run_id: u64,
    pub role: AgentRole,
    pub provider: ProviderKind,
    pub model: String,
    pub prompt: String,
    /// `"running"`, `"done"`, `"error"`, `"canceled"`.
    pub status: String,
    pub output: String,
    pub patches: Vec<ProposedPatch>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

pub type AgentNotifyFn = Arc<dyn Fn(AgentRunSnapshot) + Send + Sync>;

#[derive(Default)]
pub struct AgentOrchestrator {
    next_run_id: u64,
    runs: Arc<Mutex<Vec<AgentRunSnapshot>>>,
}

impl AgentOrchestrator {
    pub fn list_runs(&self) -> Vec<AgentRunSnapshot> {
        self.runs
            .lock()
            .map(|g| g.iter().rev().take(30).cloned().collect())
            .unwrap_or_default()
    }

    pub fn get_run(&self, run_id: u64) -> Option<AgentRunSnapshot> {
        self.runs
            .lock()
            .ok()?
            .iter()
            .find(|r| r.run_id == run_id)
            .cloned()
    }

    pub fn start_run(
        &mut self,
        workspace_root: Option<PathBuf>,
        role: AgentRole,
        config: AgentProviderConfig,
        user_prompt: String,
        qa_snapshot: Option<BrowserQaSnapshot>,
        on_update: Option<AgentNotifyFn>,
    ) -> Result<AgentRunSnapshot, String> {
        let trimmed = user_prompt.trim();
        if trimmed.is_empty() {
            return Err("agent prompt cannot be empty".to_string());
        }

        self.next_run_id += 1;
        let run_id = self.next_run_id;
        let initial = AgentRunSnapshot {
            run_id,
            role,
            provider: config.provider,
            model: config.model.clone(),
            prompt: trimmed.to_string(),
            status: "running".to_string(),
            output: format!(
                "[{}] Starting with {:?}...\n",
                role.label(),
                config.provider
            ),
            patches: Vec::new(),
            started_at: unix_now_secs(),
            finished_at: None,
        };

        if let Ok(mut guard) = self.runs.lock() {
            guard.push(initial.clone());
        }

        let runs_arc = self.runs.clone();
        let prompt_owned = trimmed.to_string();
        let _ = thread::Builder::new()
            .name(format!("seu-agent-{run_id}"))
            .spawn(move || {
                let full_prompt = build_role_prompt(
                    role,
                    &prompt_owned,
                    workspace_root.as_deref(),
                    qa_snapshot.as_ref(),
                );
                let result = execute_provider(&config, &full_prompt, workspace_root.as_deref());

                let updated = {
                    let Ok(mut guard) = runs_arc.lock() else {
                        return;
                    };
                    let Some(run) = guard.iter_mut().find(|r| r.run_id == run_id) else {
                        return;
                    };
                    match result {
                        Ok(text) => {
                            let patches = if role == AgentRole::Coder {
                                parse_proposed_patches(&text)
                            } else {
                                Vec::new()
                            };
                            run.output = text;
                            run.patches = patches;
                            run.status = "done".to_string();
                        }
                        Err(err) => {
                            run.output = format!("Error: {err}");
                            run.status = "error".to_string();
                        }
                    }
                    run.finished_at = Some(unix_now_secs());
                    run.clone()
                };

                if let Some(cb) = on_update {
                    cb(updated);
                }
            });

        Ok(initial)
    }

    pub fn approve_patch(
        &self,
        workspace_root: &Path,
        run_id: u64,
        patch_index: usize,
    ) -> Result<AgentRunSnapshot, String> {
        let (rel_path, new_content) = {
            let guard = self.runs.lock().map_err(|_| "lock error".to_string())?;
            let run = guard
                .iter()
                .find(|r| r.run_id == run_id)
                .ok_or_else(|| format!("run not found: {run_id}"))?;
            let patch = run
                .patches
                .get(patch_index)
                .ok_or_else(|| format!("patch index out of bounds: {patch_index}"))?;
            (patch.rel_path.clone(), patch.new_content.clone())
        };

        write_file_text(workspace_root, &rel_path, &new_content)
            .map_err(|e: WorkspaceError| e.to_string())?;

        let mut guard = self.runs.lock().map_err(|_| "lock error".to_string())?;
        let run = guard
            .iter_mut()
            .find(|r| r.run_id == run_id)
            .ok_or_else(|| format!("run not found: {run_id}"))?;
        if let Some(patch) = run.patches.get_mut(patch_index) {
            patch.applied = true;
        }
        Ok(run.clone())
    }
}

/// Detects locally installed agent CLIs (`gemini`, `claude`, `codex`, `ollama`)
/// in pure Rust by scanning `PATH` (zero subprocess spawns, < 1 ms).
pub fn detect_local_clis() -> Vec<DetectedLocalCli> {
    let candidates = [
        ("gemini", "Gemini CLI (gemini)"),
        ("claude", "Claude Code (claude)"),
        ("codex", "Codex CLI (codex)"),
        ("ollama", "Ollama Local (ollama)"),
    ];
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();

    #[cfg(target_os = "windows")]
    let exts: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    candidates
        .into_iter()
        .map(|(bin, label)| {
            #[cfg(target_os = "windows")]
            let available = is_in_path_windows(bin, &path_dirs, &exts);
            #[cfg(not(target_os = "windows"))]
            let available = is_in_path_unix(bin, &path_dirs);
            DetectedLocalCli {
                id: bin.to_string(),
                label: label.to_string(),
                available,
            }
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn is_in_path_windows(bin: &str, dirs: &[PathBuf], exts: &[String]) -> bool {
    for dir in dirs {
        if dir.join(bin).is_file() {
            return true;
        }
        for ext in exts {
            if dir.join(format!("{bin}{ext}")).is_file() {
                return true;
            }
        }
    }
    false
}

#[cfg(not(target_os = "windows"))]
fn is_in_path_unix(bin: &str, dirs: &[PathBuf]) -> bool {
    dirs.iter().any(|dir| dir.join(bin).is_file())
}

fn build_role_prompt(
    role: AgentRole,
    user_prompt: &str,
    workspace_root: Option<&Path>,
    qa: Option<&BrowserQaSnapshot>,
) -> String {
    let mut ctx = String::new();
    match role {
        AgentRole::Planner => {
            ctx.push_str(
                "You are the Planner agent inside Seu Navegador Studio. \
                 Produce a concise numbered implementation & QA plan.\n\n",
            );
        }
        AgentRole::Coder => {
            ctx.push_str(
                "You are the Coder agent inside Seu Navegador Studio. \
                 When proposing a file edit, wrap each full file strictly in:\n\
                 <<<FILE: relative/path/to/file>>>\n\
                 <full file content>\n\
                 <<<END_FILE>>>\n\
                 Keep explanations concise.\n\n",
            );
        }
        AgentRole::QaBrowser => {
            ctx.push_str(
                "You are the QA Browser agent inside Seu Navegador Studio, paired directly \
                 with the live browser tab. Analyze the DOM, console errors, and user flow, \
                 and report PASS/FAIL checks plus concrete bug steps.\n\n",
            );
        }
    }

    if let Some(root) = workspace_root {
        let empty_git = HashMap::new();
        if let Ok(entries) = list_directory(root, "", &empty_git) {
            let names: Vec<String> = entries
                .into_iter()
                .take(30)
                .map(|e| {
                    if e.is_dir {
                        format!("{}/", e.name)
                    } else {
                        e.name
                    }
                })
                .collect();
            ctx.push_str(&format!(
                "Workspace root: {}\nTop-level files: {}\n\n",
                root.to_string_lossy(),
                names.join(", ")
            ));
        }
        // Include Cargo.toml or package.json snippet if present for quick project grounding.
        for manifest in ["package.json", "Cargo.toml"] {
            if let Ok(file) = read_file_text(root, manifest) {
                let snippet: String = file.content.chars().take(1200).collect();
                ctx.push_str(&format!("--- {manifest} ---\n{snippet}\n\n"));
                break;
            }
        }
    }

    if let Some(snap) = qa {
        ctx.push_str(&format!(
            "Live Paired Browser Tab:\n- URL: {}\n- Title: {}\n- Viewport: {}x{}\n- Console Errors ({}): {:?}\n- Interactive elements ({}): {}\n- Visible text excerpt:\n{}\n\n",
            snap.url,
            snap.title,
            snap.viewport_w,
            snap.viewport_h,
            snap.console_errors.len(),
            snap.console_errors
                .iter()
                .take(10)
                .map(|e| e.message.as_str())
                .collect::<Vec<_>>(),
            snap.interactive_count,
            snap.interactive_summary
                .iter()
                .take(15)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | "),
            snap.text_excerpt.chars().take(1200).collect::<String>(),
        ));
    }

    ctx.push_str("User task:\n");
    ctx.push_str(user_prompt);
    ctx
}

/// Parses structured `<<<FILE: path>>> ... <<<END_FILE>>>` blocks produced by Coder agents.
pub fn parse_proposed_patches(output: &str) -> Vec<ProposedPatch> {
    let mut patches = Vec::new();
    let mut cursor = output;

    while let Some(start_idx) = cursor.find("<<<FILE:") {
        let after_marker = &cursor[start_idx + "<<<FILE:".len()..];
        let Some(header_end) = after_marker.find(">>>") else {
            break;
        };
        let rel_path = after_marker[..header_end].trim().replace('\\', "/");
        let body_start = &after_marker[header_end + 3..];
        let body_trimmed = body_start
            .strip_prefix("\r\n")
            .or_else(|| body_start.strip_prefix('\n'))
            .unwrap_or(body_start);

        let Some(end_idx) = body_trimmed.find("<<<END_FILE>>>") else {
            break;
        };
        let new_content = body_trimmed[..end_idx].to_string();
        if !rel_path.is_empty() && !rel_path.contains("..") {
            let line_count = new_content.lines().count();
            let preview = format!("{rel_path} ({line_count} lines)");
            patches.push(ProposedPatch {
                rel_path,
                new_content,
                preview,
                applied: false,
            });
        }
        cursor = &body_trimmed[end_idx + "<<<END_FILE>>>".len()..];
    }

    patches
}

fn execute_provider(
    config: &AgentProviderConfig,
    full_prompt: &str,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    match config.provider {
        ProviderKind::GeminiApi => call_gemini_api(config, full_prompt),
        ProviderKind::AnthropicApi => call_anthropic_api(config, full_prompt),
        ProviderKind::OpenAiApi => call_openai_api(config, full_prompt),
        ProviderKind::OllamaLocal => call_ollama_api(config, full_prompt),
        ProviderKind::CliGemini => run_local_cli("gemini", &["-p", full_prompt], workspace_root),
        ProviderKind::CliClaude => run_local_cli("claude", &["-p", full_prompt], workspace_root),
        ProviderKind::CliCodex => run_local_cli("codex", &["exec", full_prompt], workspace_root),
        ProviderKind::CliCustom => {
            run_custom_cli(&config.custom_cli_cmd, full_prompt, workspace_root)
        }
    }
}

fn run_local_cli(
    bin: &str,
    args: &[&str],
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let mut cmd = Command::new(bin);
    cmd.args(args);
    if let Some(cwd) = workspace_root {
        cmd.current_dir(cwd);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("failed to launch local CLI '{bin}': {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if output.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

fn run_custom_cli(
    template: &str,
    full_prompt: &str,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let trimmed = template.trim();
    if trimmed.is_empty() {
        return Err("custom CLI command is empty".to_string());
    }
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
    if let Some(cwd) = workspace_root {
        cmd.current_dir(cwd);
    }
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to spawn custom CLI: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(full_prompt.as_bytes());
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("custom CLI error: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if out.status.success() {
        Ok(if stdout.is_empty() { stderr } else { stdout })
    } else {
        Err(if stderr.is_empty() { stdout } else { stderr })
    }
}

fn curl_post_json(
    url: &str,
    headers: &[(&str, String)],
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let curl_bin = if cfg!(target_os = "windows") {
        "curl.exe"
    } else {
        "curl"
    };
    let mut cmd = Command::new(curl_bin);
    cmd.args(["-sS", "--max-time", "90", "-X", "POST", url]);
    cmd.args(["-H", "Content-Type: application/json"]);
    for (k, v) in headers {
        cmd.arg("-H").arg(format!("{k}: {v}"));
    }
    cmd.args(["--data-binary", "@-"]);

    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("failed to invoke curl: {e}"))?;

    let payload = serde_json::to_vec(body).map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&payload)
            .map_err(|e| format!("failed writing HTTP body: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("HTTP wait error: {e}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("HTTP error: {} {}", stderr.trim(), stdout.trim()));
    }
    serde_json::from_str(&stdout).map_err(|e| {
        format!(
            "invalid JSON from provider ({e}): {}",
            stdout.chars().take(240).collect::<String>()
        )
    })
}

fn call_gemini_api(config: &AgentProviderConfig, prompt: &str) -> Result<String, String> {
    if config.api_key.trim().is_empty() {
        return Err("Gemini API key is required".to_string());
    }
    let model = if config.model.trim().is_empty() {
        "gemini-2.5-flash"
    } else {
        config.model.trim()
    };
    let url =
        format!("https://generativelanguage.googleapis.com/v1beta/models/{model}:generateContent");
    let body = serde_json::json!({
        "contents": [{ "parts": [{ "text": prompt }] }]
    });
    let json = curl_post_json(
        &url,
        &[("x-goog-api-key", config.api_key.trim().to_string())],
        &body,
    )?;
    if let Some(err) = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(err.to_string());
    }
    json.pointer("/candidates/0/content/parts/0/text")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "empty response from Gemini API".to_string())
}

fn call_anthropic_api(config: &AgentProviderConfig, prompt: &str) -> Result<String, String> {
    if config.api_key.trim().is_empty() {
        return Err("Anthropic API key is required".to_string());
    }
    let model = if config.model.trim().is_empty() {
        "claude-3-7-sonnet-latest"
    } else {
        config.model.trim()
    };
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": prompt }]
    });
    let json = curl_post_json(
        "https://api.anthropic.com/v1/messages",
        &[
            ("x-api-key", config.api_key.trim().to_string()),
            ("anthropic-version", "2023-06-01".to_string()),
        ],
        &body,
    )?;
    if let Some(err) = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(err.to_string());
    }
    json.pointer("/content/0/text")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "empty response from Anthropic API".to_string())
}

fn call_openai_api(config: &AgentProviderConfig, prompt: &str) -> Result<String, String> {
    if config.api_key.trim().is_empty() {
        return Err("OpenAI API key is required".to_string());
    }
    let base = if config.base_url.trim().is_empty() {
        "https://api.openai.com/v1"
    } else {
        config.base_url.trim().trim_end_matches('/')
    };
    let model = if config.model.trim().is_empty() {
        "gpt-4.1-mini"
    } else {
        config.model.trim()
    };
    let url = format!("{base}/chat/completions");
    let body = serde_json::json!({
        "model": model,
        "messages": [{ "role": "user", "content": prompt }]
    });
    let json = curl_post_json(
        &url,
        &[("Authorization", format!("Bearer {}", config.api_key.trim()))],
        &body,
    )?;
    if let Some(err) = json
        .get("error")
        .and_then(|e| e.get("message"))
        .and_then(|m| m.as_str())
    {
        return Err(err.to_string());
    }
    json.pointer("/choices/0/message/content")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "empty response from OpenAI API".to_string())
}

fn call_ollama_api(config: &AgentProviderConfig, prompt: &str) -> Result<String, String> {
    let base = if config.base_url.trim().is_empty() {
        "http://127.0.0.1:11434"
    } else {
        config.base_url.trim().trim_end_matches('/')
    };
    let model = if config.model.trim().is_empty() {
        "qwen2.5-coder:7b"
    } else {
        config.model.trim()
    };
    let url = format!("{base}/api/chat");
    let body = serde_json::json!({
        "model": model,
        "stream": false,
        "messages": [{ "role": "user", "content": prompt }]
    });
    let json = curl_post_json(&url, &[], &body)?;
    if let Some(err) = json.get("error").and_then(|e| e.as_str()) {
        return Err(err.to_string());
    }
    json.pointer("/message/content")
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "empty response from Ollama (is Ollama running?)".to_string())
}

fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_coder_file_patches_and_blocks_traversal() {
        let output = "Updated two files:\n\
<<<FILE: src/hello.ts>>>\n\
export const hi = 'y2k';\n\
<<<END_FILE>>>\n\
<<<FILE: ../../evil.txt>>>\n\
nope\n\
<<<END_FILE>>>\n";
        let patches = parse_proposed_patches(output);
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].rel_path, "src/hello.ts");
        assert!(patches[0].new_content.contains("y2k"));
        assert!(!patches[0].applied);
    }

    #[test]
    fn runs_custom_cli_agent_and_approves_patch_inside_sandbox() {
        let tmp = std::env::temp_dir().join(format!("seu-agent-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        assert!(std::fs::create_dir_all(&tmp).is_ok());

        let mut orch = AgentOrchestrator::default();
        #[cfg(target_os = "windows")]
        let cmd = "Write-Output '<<<FILE: out.txt>>>'; Write-Output 'agent-ok'; Write-Output '<<<END_FILE>>>'";
        #[cfg(not(target_os = "windows"))]
        let cmd = "printf '<<<FILE: out.txt>>>\\nagent-ok\\n<<<END_FILE>>>\\n'";

        let cfg = AgentProviderConfig {
            provider: ProviderKind::CliCustom,
            model: "custom".into(),
            api_key: String::new(),
            base_url: String::new(),
            custom_cli_cmd: cmd.into(),
        };
        let started = orch.start_run(
            Some(tmp.clone()),
            AgentRole::Coder,
            cfg,
            "create out.txt".into(),
            None,
            None,
        );
        assert!(started.is_ok());
        if let Ok(run) = started {
            for _ in 0..50 {
                if let Some(cur) = orch.get_run(run.run_id) {
                    if cur.status != "running" {
                        assert_eq!(cur.status, "done");
                        assert_eq!(cur.patches.len(), 1);
                        let approved = orch.approve_patch(&tmp, run.run_id, 0);
                        assert!(approved.is_ok());
                        let saved =
                            std::fs::read_to_string(tmp.join("out.txt")).unwrap_or_default();
                        assert!(saved.contains("agent-ok"));
                        break;
                    }
                }
                thread::sleep(std::time::Duration::from_millis(60));
            }
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
