use std::{process::Stdio, time::Duration};

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;
use tokio::{process::Command, time::timeout};

use crate::state::CodexSettings;

const SESSION_PREAMBLE: &str = r#"你是部署在这台服务器上的 Codex，通过 Telegram 机器人只与服务器所有者交流。

规则：
1. 把当前 Telegram 对话视为一条独立会话；不要引用或猜测其他 Codex 会话、Telegram 聊天或无关任务的上下文。
2. 当前工作目录是 {workdir}。你可以按用户指令检查、修改和管理这台服务器，但要遵守正常的安全边界，避免无关或破坏性操作。
3. 回复会转换为 Telegram 富文本：使用简短标题、分组、项目符号、粗体、行内代码和代码块提高可读性。不要使用 Markdown 表格；表格内容改写成适合手机阅读的分组列表。必要时明确报告完成结果、风险或阻塞。
4. 用户可能连续追问；保留本会话中与当前任务相关的上下文。
5. 用户只通过 Telegram 操作，通常无法使用 SSH。对于查看服务、状态、日志、文件等检查任务，必须直接使用服务器工具完成并返回结果；不要要求用户自行运行 shell 命令，也不要把可由你执行的步骤推给用户。
6. 对于明确要求修复、修改或部署的任务，直接完成范围内的操作和非破坏性验证。只有涉及破坏性操作、外部写入、付费行为或会实质改变结果的缺失选择时才向用户确认。
7. 如果某个工具失败，先自行诊断并尝试安全的替代方法；仍无法完成时再报告准确的技术阻塞，不要假装已经检查成功。

用户消息：
"#;

#[derive(Clone, Debug)]
pub struct CodexRunner {
    codex_bin: std::path::PathBuf,
    codex_home: std::path::PathBuf,
    workdir: std::path::PathBuf,
    timeout: Duration,
    dangerously_bypass_approvals_and_sandbox: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodexResult {
    pub thread_id: Option<String>,
    pub response: String,
}

impl CodexRunner {
    pub fn new(
        codex_bin: std::path::PathBuf,
        codex_home: std::path::PathBuf,
        workdir: std::path::PathBuf,
        timeout: Duration,
        dangerously_bypass_approvals_and_sandbox: bool,
    ) -> Self {
        Self {
            codex_bin,
            codex_home,
            workdir,
            timeout,
            dangerously_bypass_approvals_and_sandbox,
        }
    }

    pub async fn run(
        &self,
        thread_id: Option<&str>,
        user_message: &str,
        settings: &CodexSettings,
    ) -> Result<CodexResult> {
        let mut command = Command::new(&self.codex_bin);
        command
            .env("CODEX_HOME", &self.codex_home)
            // Codex never needs Telegram credentials. Avoid leaking them through
            // ordinary subprocess environment inspection.
            .env_remove("TELOXIDE_TOKEN")
            .env_remove("TERMPIGEON_OWNER_USER_ID")
            .current_dir(&self.workdir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        if self.dangerously_bypass_approvals_and_sandbox {
            command.arg("--dangerously-bypass-approvals-and-sandbox");
        } else {
            command
                .args(["--sandbox", "workspace-write"])
                .arg("-c")
                .arg("approval_policy=\"never\"");
        }

        match thread_id {
            Some(thread_id) => {
                validate_thread_id(thread_id)?;
                command.args([
                    "exec",
                    "resume",
                    "--json",
                    "--skip-git-repo-check",
                    "--thread-source",
                    "telegram",
                ]);
                apply_settings(&mut command, settings);
                command.arg(thread_id).arg(user_message);
            }
            None => {
                let prompt = SESSION_PREAMBLE
                    .replace("{workdir}", &self.workdir.display().to_string())
                    + user_message;
                command.args([
                    "exec",
                    "--json",
                    "--skip-git-repo-check",
                    "--thread-source",
                    "telegram",
                ]);
                apply_settings(&mut command, settings);
                command.arg("-C").arg(&self.workdir).arg(prompt);
            }
        }

        let output = timeout(self.timeout, command.output())
            .await
            .map_err(|_| anyhow!("Codex timed out after {} seconds", self.timeout.as_secs()))?
            .context("start Codex process")?;
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let parsed = parse_jsonl(&stdout);

        if !output.status.success() {
            let detail = concise_error(&stderr, &stdout);
            bail!("Codex exited with {}: {detail}", output.status);
        }
        if parsed.response.trim().is_empty() {
            let detail = concise_error(&stderr, &stdout);
            bail!("Codex returned no assistant message: {detail}");
        }
        Ok(parsed)
    }
}

fn apply_settings(command: &mut Command, settings: &CodexSettings) {
    command
        .arg("--model")
        .arg(settings.model.id())
        .arg("-c")
        .arg(format!(
            "model_reasoning_effort=\"{}\"",
            settings.reasoning_effort.id()
        ))
        .arg("-c")
        .arg(format!("model_verbosity=\"{}\"", settings.verbosity.id()));
    if settings.fast_mode {
        command
            .arg("-c")
            .arg("service_tier=\"fast\"")
            .arg("-c")
            .arg("features.fast_mode=true");
    } else {
        command
            .arg("-c")
            .arg("service_tier=\"default\"")
            .arg("-c")
            .arg("features.fast_mode=false");
    }
}

fn validate_thread_id(value: &str) -> Result<()> {
    let valid = value.len() >= 32
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    if !valid {
        bail!("stored Codex thread id is invalid");
    }
    Ok(())
}

fn parse_jsonl(stdout: &str) -> CodexResult {
    let mut result = CodexResult::default();
    let mut messages = Vec::new();

    for line in stdout.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("thread.started") => {
                if let Some(thread_id) = event.get("thread_id").and_then(Value::as_str) {
                    result.thread_id = Some(thread_id.to_owned());
                }
            }
            Some("item.completed") => {
                let item = &event["item"];
                if item.get("type").and_then(Value::as_str) == Some("agent_message")
                    && let Some(text) = item.get("text").and_then(Value::as_str)
                    && !text.trim().is_empty()
                {
                    messages.push(text.trim().to_owned());
                }
            }
            _ => {}
        }
    }
    result.response = messages.join("\n\n");
    result
}

fn concise_error(stderr: &str, stdout: &str) -> String {
    let source = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    let mut text = source.trim().replace('\0', "");
    const MAX_CHARS: usize = 1200;
    if text.chars().count() > MAX_CHARS {
        text = text
            .chars()
            .rev()
            .take(MAX_CHARS)
            .collect::<String>()
            .chars()
            .rev()
            .collect();
        text.insert(0, '…');
    }
    if text.is_empty() {
        "no diagnostic output".to_owned()
    } else {
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thread_and_assistant_messages() {
        let jsonl = r#"{"type":"thread.started","thread_id":"0199-aaaa-bbbb-cccc-dddddddddddd"}
{"type":"item.completed","item":{"type":"command_execution","text":"ignored"}}
{"type":"item.completed","item":{"type":"agent_message","text":"first"}}
{"type":"item.completed","item":{"type":"agent_message","text":"second"}}"#;
        let parsed = parse_jsonl(jsonl);
        assert_eq!(
            parsed.thread_id.as_deref(),
            Some("0199-aaaa-bbbb-cccc-dddddddddddd")
        );
        assert_eq!(parsed.response, "first\n\nsecond");
    }

    #[test]
    fn rejects_malformed_thread_ids() {
        assert!(validate_thread_id("--last").is_err());
        assert!(validate_thread_id("abcd/../../../etc/passwd").is_err());
    }
}
