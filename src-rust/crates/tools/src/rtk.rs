//! Native RTK (Rust Token Killer) adapter.
//!
//! Claurst treats RTK as an optional shell-output compression accelerator.  The
//! adapter never executes the user's command directly; it only asks `rtk
//! rewrite <command>` for an optimized equivalent, then the Bash/PtyBash tool
//! executes either the original or rewritten command through the normal
//! permission and shell-state path.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use claurst_core::bash_classifier::{classify_bash_command, BashRiskLevel};
use claurst_core::config::{RtkConfig, RtkMode};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tracing::{debug, warn};

const MIN_REWRITE_TIMEOUT_MS: u64 = 100;
const MAX_REWRITE_TIMEOUT_MS: u64 = 10_000;

/// RTK rewrite decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtkRewriteDecision {
    pub original_command: String,
    pub effective_command: String,
    pub status: RtkRewriteStatus,
    pub detail: Option<String>,
}

impl RtkRewriteDecision {
    fn passthrough(command: &str, status: RtkRewriteStatus, detail: Option<String>) -> Self {
        Self {
            original_command: command.to_string(),
            effective_command: command.to_string(),
            status,
            detail,
        }
    }

    fn changed(command: &str, rewritten: String, status: RtkRewriteStatus) -> Self {
        Self {
            original_command: command.to_string(),
            effective_command: rewritten,
            status,
            detail: None,
        }
    }

    pub fn was_rewritten(&self) -> bool {
        matches!(
            self.status,
            RtkRewriteStatus::Rewritten | RtkRewriteStatus::RewrittenAfterAsk
        )
    }
}

/// RTK rewrite status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtkRewriteStatus {
    Disabled,
    Excluded,
    Missing,
    Timeout,
    Error,
    NoRewrite,
    Denied,
    Suggested,
    Rewritten,
    RewrittenAfterAsk,
}

impl RtkRewriteStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Excluded => "excluded",
            Self::Missing => "missing",
            Self::Timeout => "timeout",
            Self::Error => "error",
            Self::NoRewrite => "no-rewrite",
            Self::Denied => "denied",
            Self::Suggested => "suggested",
            Self::Rewritten => "rewritten",
            Self::RewrittenAfterAsk => "rewritten-after-ask",
        }
    }
}

/// Ask RTK to rewrite a Bash command according to the supplied config.
pub async fn rewrite_bash_command(
    command: &str,
    config: &RtkConfig,
    working_dir: &Path,
) -> RtkRewriteDecision {
    let trimmed = command.trim();
    if trimmed.is_empty() || !config.enabled || matches!(config.mode, RtkMode::Off) {
        return RtkRewriteDecision::passthrough(command, RtkRewriteStatus::Disabled, None);
    }

    if is_env_disabled() {
        return RtkRewriteDecision::passthrough(
            command,
            RtkRewriteStatus::Disabled,
            Some("disabled by CLAURST_RTK".to_string()),
        );
    }

    if trimmed == "rtk" || trimmed.starts_with("rtk ") {
        return RtkRewriteDecision::passthrough(command, RtkRewriteStatus::NoRewrite, None);
    }

    if is_excluded(trimmed, &config.exclude_commands) {
        return RtkRewriteDecision::passthrough(command, RtkRewriteStatus::Excluded, None);
    }

    let Some(binary) = resolve_binary(&config.binary) else {
        return RtkRewriteDecision::passthrough(
            command,
            RtkRewriteStatus::Missing,
            Some(format!("{} not found", config.binary)),
        );
    };

    let timeout_ms = config
        .rewrite_timeout_ms
        .clamp(MIN_REWRITE_TIMEOUT_MS, MAX_REWRITE_TIMEOUT_MS);
    let mut child = Command::new(binary);
    child
        .arg("rewrite")
        .arg(command)
        .current_dir(working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), child.output()).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => {
            return RtkRewriteDecision::passthrough(
                command,
                RtkRewriteStatus::Error,
                Some(err.to_string()),
            );
        }
        Err(_) => return RtkRewriteDecision::passthrough(command, RtkRewriteStatus::Timeout, None),
    };

    let rewritten = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    match exit_code {
        0 => apply_rewrite(command, rewritten, config.mode, RtkRewriteStatus::Rewritten),
        1 => RtkRewriteDecision::passthrough(command, RtkRewriteStatus::NoRewrite, None),
        2 => RtkRewriteDecision::passthrough(
            command,
            RtkRewriteStatus::Denied,
            (!stderr.is_empty()).then_some(stderr),
        ),
        3 => apply_rewrite(
            command,
            rewritten,
            config.mode,
            RtkRewriteStatus::RewrittenAfterAsk,
        ),
        _ => RtkRewriteDecision::passthrough(
            command,
            RtkRewriteStatus::Error,
            Some(if stderr.is_empty() {
                format!("rtk rewrite exited with status {exit_code}")
            } else {
                stderr
            }),
        ),
    }
}

fn apply_rewrite(
    command: &str,
    rewritten: String,
    mode: RtkMode,
    rewrite_status: RtkRewriteStatus,
) -> RtkRewriteDecision {
    if rewritten.is_empty() || rewritten == command.trim() {
        return RtkRewriteDecision::passthrough(command, RtkRewriteStatus::NoRewrite, None);
    }

    match mode {
        RtkMode::Off => RtkRewriteDecision::passthrough(command, RtkRewriteStatus::Disabled, None),
        RtkMode::Suggest => {
            let mut decision =
                RtkRewriteDecision::passthrough(command, RtkRewriteStatus::Suggested, None);
            decision.detail = Some(rewritten);
            decision
        }
        RtkMode::Rewrite => RtkRewriteDecision::changed(command, rewritten, rewrite_status),
    }
}

/// Classify the effective command, treating `rtk <cmd>` as `<cmd>` for safety.
pub fn classify_effective_bash_command(command: &str) -> BashRiskLevel {
    let trimmed = command.trim();
    if let Some(rest) = trimmed.strip_prefix("rtk ") {
        return classify_bash_command(rest);
    }
    classify_bash_command(trimmed)
}

pub fn attach_rtk_metadata(mut result: ToolResult, decision: &RtkRewriteDecision) -> ToolResult {
    if matches!(
        decision.status,
        RtkRewriteStatus::Disabled | RtkRewriteStatus::NoRewrite | RtkRewriteStatus::Missing
    ) {
        return result;
    }

    result.metadata = Some(json!({
        "rtk": {
            "status": decision.status.as_str(),
            "original_command": decision.original_command,
            "effective_command": decision.effective_command,
            "detail": decision.detail,
        }
    }));
    result
}

pub async fn maybe_rewrite_for_tool(
    command: &str,
    ctx: &ToolContext,
    tool_name: &str,
) -> RtkRewriteDecision {
    let decision = rewrite_bash_command(command, &ctx.config.rtk, &ctx.working_dir).await;
    match decision.status {
        RtkRewriteStatus::Rewritten | RtkRewriteStatus::RewrittenAfterAsk => {
            debug!(
                tool = tool_name,
                original = %decision.original_command,
                rewritten = %decision.effective_command,
                status = decision.status.as_str(),
                "RTK rewrote bash command"
            );
        }
        RtkRewriteStatus::Error | RtkRewriteStatus::Timeout | RtkRewriteStatus::Denied => {
            warn!(
                tool = tool_name,
                command = %decision.original_command,
                status = decision.status.as_str(),
                detail = ?decision.detail,
                "RTK did not rewrite bash command"
            );
        }
        _ => {
            debug!(
                tool = tool_name,
                command = %decision.original_command,
                status = decision.status.as_str(),
                "RTK passthrough"
            );
        }
    }
    decision
}

fn is_env_disabled() -> bool {
    std::env::var("CLAURST_RTK")
        .ok()
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "0" | "false" | "off" | "no" | "disabled"
            )
        })
        .unwrap_or(false)
}

fn is_excluded(command: &str, excludes: &[String]) -> bool {
    excludes.iter().any(|exclude| {
        let exclude = exclude.trim();
        !exclude.is_empty() && (command == exclude || command.starts_with(&format!("{exclude} ")))
    })
}

fn resolve_binary(binary: &str) -> Option<PathBuf> {
    let binary = binary.trim();
    if binary.is_empty() {
        return None;
    }

    let path = PathBuf::from(binary);
    if path.is_absolute() || binary.contains(std::path::MAIN_SEPARATOR) {
        return Some(path);
    }

    which::which(binary).ok()
}

#[derive(Debug, Deserialize)]
struct RtkToolInput {
    #[serde(default = "default_rtk_action")]
    action: String,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    timeout_ms: Option<u64>,
    #[serde(default)]
    max_output_chars: Option<usize>,
}

fn default_rtk_action() -> String {
    "status".to_string()
}

pub struct RtkTool;

#[async_trait]
impl Tool for RtkTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_RTK
    }

    fn description(&self) -> &str {
        "Inspect and validate the native RTK integration. Actions: status, rewrite, gain. RTK is used by Bash/PtyBash to compact noisy shell command output when enabled."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["status", "rewrite", "gain"], "description": "RTK action; defaults to status"},
                "command": {"type": "string", "description": "Bash command for action=rewrite"},
                "timeout_ms": {"type": "number", "description": "timeout in milliseconds; default 30000"},
                "max_output_chars": {"type": "number", "description": "maximum returned characters; default 60000"}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: RtkToolInput = match serde_json::from_value(input) {
            Ok(params) => params,
            Err(err) => return ToolResult::error(format!("Invalid input: {err}")),
        };

        if let Err(err) = ctx.check_permission_for_path(
            self.name(),
            "Inspect native RTK integration",
            ctx.working_dir.clone(),
            true,
        ) {
            return ToolResult::error(err.to_string());
        }

        match params.action.as_str() {
            "status" => rtk_status(ctx).await,
            "rewrite" => {
                let Some(command) = params.command.as_deref() else {
                    return ToolResult::error("command is required for action=rewrite");
                };
                let decision =
                    rewrite_bash_command(command, &ctx.config.rtk, &ctx.working_dir).await;
                ToolResult::success(format!(
                    "rtk status: {}\noriginal: {}\neffective: {}\n{}",
                    decision.status.as_str(),
                    decision.original_command,
                    decision.effective_command,
                    decision
                        .detail
                        .map(|detail| format!("detail: {detail}"))
                        .unwrap_or_default()
                ))
            }
            "gain" => run_rtk_gain(ctx, params.timeout_ms, params.max_output_chars).await,
            other => ToolResult::error(format!("unsupported rtk action: {other}")),
        }
    }
}

async fn rtk_status(ctx: &ToolContext) -> ToolResult {
    let config = &ctx.config.rtk;
    let binary = resolve_binary(&config.binary);
    let version = if let Some(binary_path) = &binary {
        Command::new(binary_path)
            .arg("--version")
            .current_dir(&ctx.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .filter(|value| !value.is_empty())
    } else {
        None
    };

    ToolResult::success(format!(
        "rtk.enabled: {}\nrtk.mode: {:?}\nrtk.binary: {}\nrtk.available: {}\nrtk.version: {}",
        config.enabled,
        config.mode,
        config.binary,
        binary.is_some(),
        version.unwrap_or_else(|| "(unavailable)".to_string())
    ))
}

async fn run_rtk_gain(
    ctx: &ToolContext,
    timeout_ms: Option<u64>,
    max_output_chars: Option<usize>,
) -> ToolResult {
    let Some(binary) = resolve_binary(&ctx.config.rtk.binary) else {
        return ToolResult::error(format!("{} not found", ctx.config.rtk.binary));
    };

    let timeout_ms = timeout_ms.unwrap_or(30_000).clamp(100, 120_000);
    let output = match tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        Command::new(binary)
            .arg("gain")
            .current_dir(&ctx.working_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => output,
        Ok(Err(err)) => return ToolResult::error(format!("failed to run rtk gain: {err}")),
        Err(_) => return ToolResult::error(format!("rtk gain timed out after {timeout_ms}ms")),
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    if !output.stderr.is_empty() {
        if !combined.is_empty() {
            combined.push_str("\n--- stderr ---\n");
        }
        combined.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    if combined.is_empty() {
        combined.push_str("[no output]");
    }

    let max_output_chars = max_output_chars.unwrap_or(60_000);
    if combined.chars().count() > max_output_chars {
        combined = combined.chars().take(max_output_chars).collect::<String>();
        combined.push_str("\n\n[output truncated]");
    }

    if output.status.success() {
        ToolResult::success(combined)
    } else {
        ToolResult::error(format!(
            "rtk gain exited with status {}\n{combined}",
            output.status
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_rtk_wrapped_commands_by_inner_command() {
        assert_eq!(
            classify_effective_bash_command("rtk rm -rf /"),
            BashRiskLevel::Critical
        );
        assert_eq!(
            classify_effective_bash_command("rtk git status"),
            BashRiskLevel::Safe
        );
    }

    #[tokio::test]
    async fn missing_binary_passes_through() {
        let config = RtkConfig {
            binary: "/definitely/missing/rtk".to_string(),
            ..RtkConfig::default()
        };
        let decision = rewrite_bash_command("git status", &config, Path::new(".")).await;
        assert_eq!(decision.status, RtkRewriteStatus::Error);
        assert_eq!(decision.effective_command, "git status");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rewrite_exit_zero_changes_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let rtk = temp.path().join("rtk");
        std::fs::write(
            &rtk,
            "#!/usr/bin/env bash\nif [ \"$1\" = rewrite ]; then echo \"rtk git status\"; exit 0; fi\n",
        )
        .expect("write fake rtk");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rtk, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake rtk");

        let config = RtkConfig {
            binary: rtk.to_string_lossy().into_owned(),
            ..RtkConfig::default()
        };
        let decision = rewrite_bash_command("git status", &config, temp.path()).await;
        assert_eq!(decision.status, RtkRewriteStatus::Rewritten);
        assert_eq!(decision.effective_command, "rtk git status");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn suggest_mode_does_not_change_effective_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let rtk = temp.path().join("rtk");
        std::fs::write(
            &rtk,
            "#!/usr/bin/env bash\nif [ \"$1\" = rewrite ]; then echo \"rtk cargo test\"; exit 0; fi\n",
        )
        .expect("write fake rtk");
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&rtk, std::fs::Permissions::from_mode(0o755))
            .expect("chmod fake rtk");

        let config = RtkConfig {
            binary: rtk.to_string_lossy().into_owned(),
            mode: RtkMode::Suggest,
            ..RtkConfig::default()
        };
        let decision = rewrite_bash_command("cargo test", &config, temp.path()).await;
        assert_eq!(decision.status, RtkRewriteStatus::Suggested);
        assert_eq!(decision.effective_command, "cargo test");
        assert_eq!(decision.detail.as_deref(), Some("rtk cargo test"));
    }
}
