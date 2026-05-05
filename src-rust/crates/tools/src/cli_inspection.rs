// CLI-backed codebase inspection tools for local developer workflows.

use crate::{PermissionLevel, Tool, ToolContext, ToolResult};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{json, Value};
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;

const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_MAX_OUTPUT_CHARS: usize = 60_000;

#[derive(Debug, Deserialize)]
struct CliToolInput {
    #[serde(default)]
    action: Option<String>,
    #[serde(default)]
    query: Option<String>,
    #[serde(default)]
    question: Option<String>,
    #[serde(default)]
    queries: Vec<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default = "default_timeout_ms")]
    timeout_ms: u64,
    #[serde(default = "default_max_output_chars")]
    max_output_chars: usize,
}

fn default_timeout_ms() -> u64 {
    DEFAULT_TIMEOUT_MS
}

fn default_max_output_chars() -> usize {
    DEFAULT_MAX_OUTPUT_CHARS
}

fn truncate_chars(mut text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }

    let mut truncated: String = text.chars().take(max_chars).collect();
    truncated.push_str("\n\n[output truncated]");
    text.clear();
    truncated
}

async fn run_cli(
    program: &str,
    args: &[String],
    ctx: &ToolContext,
    timeout_ms: u64,
    max_output_chars: usize,
) -> ToolResult {
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(&ctx.working_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = match tokio::time::timeout(Duration::from_millis(timeout_ms), cmd.output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return ToolResult::error(format!("failed to run {program}: {e}")),
        Err(_) => return ToolResult::error(format!("{program} timed out after {timeout_ms}ms")),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut combined = String::new();
    if !stdout.is_empty() {
        combined.push_str(&stdout);
    }
    if !stderr.is_empty() {
        if !combined.is_empty() {
            combined.push_str("\n--- stderr ---\n");
        }
        combined.push_str(&stderr);
    }
    if combined.is_empty() {
        combined.push_str("[no output]");
    }

    let combined = truncate_chars(combined, max_output_chars);
    if output.status.success() {
        ToolResult::success(combined)
    } else {
        ToolResult::error(format!(
            "{program} exited with status {}\n{}",
            output.status, combined
        ))
    }
}

fn fffq_args(params: &CliToolInput) -> Result<Vec<String>, String> {
    if !params.args.is_empty() {
        return Ok(params.args.clone());
    }

    match params.action.as_deref().unwrap_or("grep") {
        "ensure" => Ok(vec!["ensure".to_string()]),
        "find" => params
            .query
            .as_ref()
            .map(|query| vec!["find".to_string(), query.clone()])
            .ok_or_else(|| "query is required for fffq find".to_string()),
        "grep" => params
            .query
            .as_ref()
            .map(|query| vec!["grep".to_string(), query.clone()])
            .ok_or_else(|| "query is required for fffq grep".to_string()),
        "multi-grep" | "multi_grep" => {
            if params.queries.is_empty() {
                return Err("queries is required for fffq multi-grep".to_string());
            }
            let mut args = vec!["multi-grep".to_string()];
            args.extend(params.queries.clone());
            Ok(args)
        }
        other => Err(format!("unsupported fffq action: {other}")),
    }
}

fn graphifyq_args(params: &CliToolInput) -> Result<Vec<String>, String> {
    if !params.args.is_empty() {
        return Ok(params.args.clone());
    }

    match params.action.as_deref().unwrap_or("query") {
        "ensure" => Ok(vec!["ensure".to_string(), "--no-auto-refresh".to_string()]),
        "query" => params
            .question
            .as_ref()
            .or(params.query.as_ref())
            .map(|question| vec!["query".to_string(), question.clone()])
            .ok_or_else(|| "question is required for graphifyq query".to_string()),
        other => Err(format!("unsupported graphifyq action: {other}")),
    }
}

pub struct FffqTool;

#[async_trait]
impl Tool for FffqTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_FFFQ
    }

    fn description(&self) -> &str {
        "Use fffq for repository inspection. Prefer this before Grep/Glob/Bash for file discovery, exact text lookup, and symbol lookup. Actions: ensure, find, grep, multi-grep."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["ensure", "find", "grep", "multi-grep", "multi_grep"], "description": "fffq action; defaults to grep"},
                "query": {"type": "string", "description": "query for find/grep"},
                "queries": {"type": "array", "items": {"type": "string"}, "description": "queries for multi-grep"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "raw fffq arguments; overrides action/query"},
                "timeout_ms": {"type": "number", "description": "timeout in milliseconds; default 30000"},
                "max_output_chars": {"type": "number", "description": "maximum returned characters; default 60000"}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: CliToolInput = match serde_json::from_value(input) {
            Ok(params) => params,
            Err(e) => return ToolResult::error(format!("Invalid input: {e}")),
        };
        if let Err(e) = ctx.check_permission_for_path(
            self.name(),
            "Inspect repository with fffq",
            ctx.working_dir.clone(),
            true,
        ) {
            return ToolResult::error(e.to_string());
        }
        let args = match fffq_args(&params) {
            Ok(args) => args,
            Err(e) => return ToolResult::error(e),
        };
        run_cli(
            "fffq",
            &args,
            ctx,
            params.timeout_ms,
            params.max_output_chars,
        )
        .await
    }
}

pub struct GraphifyqTool;

#[async_trait]
impl Tool for GraphifyqTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_GRAPHIFYQ
    }

    fn description(&self) -> &str {
        "Use graphifyq for architecture/data-flow/dependency questions after FFF. Prefer action=query for existing graphs; action=ensure checks graph availability without auto-refresh by default."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["query", "ensure"], "description": "graphifyq action; defaults to query"},
                "question": {"type": "string", "description": "architecture/data-flow question for graphifyq query"},
                "query": {"type": "string", "description": "alias for question"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "raw graphifyq arguments; overrides action/question"},
                "timeout_ms": {"type": "number", "description": "timeout in milliseconds; default 30000"},
                "max_output_chars": {"type": "number", "description": "maximum returned characters; default 60000"}
            }
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let params: CliToolInput = match serde_json::from_value(input) {
            Ok(params) => params,
            Err(e) => return ToolResult::error(format!("Invalid input: {e}")),
        };
        if let Err(e) = ctx.check_permission_for_path(
            self.name(),
            "Inspect repository architecture with graphifyq",
            ctx.working_dir.clone(),
            true,
        ) {
            return ToolResult::error(e.to_string());
        }
        let args = match graphifyq_args(&params) {
            Ok(args) => args,
            Err(e) => return ToolResult::error(e),
        };
        run_cli(
            "graphifyq",
            &args,
            ctx,
            params.timeout_ms,
            params.max_output_chars,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fffq_find_args_from_query() {
        let input = CliToolInput {
            action: Some("find".to_string()),
            query: Some("codex".to_string()),
            question: None,
            queries: vec![],
            args: vec![],
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
        };
        assert_eq!(fffq_args(&input).unwrap(), vec!["find", "codex"]);
    }

    #[test]
    fn graphify_query_args_from_question() {
        let input = CliToolInput {
            action: Some("query".to_string()),
            query: None,
            question: Some("how are tools wired?".to_string()),
            queries: vec![],
            args: vec![],
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
        };
        assert_eq!(
            graphifyq_args(&input).unwrap(),
            vec!["query", "how are tools wired?"]
        );
    }
}
