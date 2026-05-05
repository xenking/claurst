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
    #[serde(default)]
    db: Option<String>,
    #[serde(default)]
    embedder: Option<String>,
    #[serde(default)]
    dimension: Option<u64>,
    #[serde(default)]
    model2vec_model: Option<String>,
    #[serde(default)]
    project_path: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    importance: Option<f64>,
    #[serde(default)]
    memory_id: Option<String>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    agent: Option<String>,
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

fn push_omx_memory_global_args(args: &mut Vec<String>, params: &CliToolInput) {
    if let Some(db) = &params.db {
        args.extend(["--db".to_string(), db.clone()]);
    }
    if let Some(embedder) = &params.embedder {
        args.extend(["--embedder".to_string(), embedder.clone()]);
    }
    if let Some(dimension) = params.dimension {
        args.extend(["--dimension".to_string(), dimension.to_string()]);
    }
    if let Some(model) = &params.model2vec_model {
        args.extend(["--model2vec-model".to_string(), model.clone()]);
    }
}

fn omx_memory_args(params: &CliToolInput) -> Result<Vec<String>, String> {
    if !params.args.is_empty() {
        return Ok(params.args.clone());
    }

    let mut args = Vec::new();
    push_omx_memory_global_args(&mut args, params);

    match params.action.as_deref().unwrap_or("retrieve") {
        "retrieve" => {
            let query = params
                .query
                .as_ref()
                .ok_or_else(|| "query is required for omx-memory retrieve".to_string())?;
            args.extend(["retrieve".to_string(), query.clone()]);
            if let Some(project_path) = &params.project_path {
                args.extend(["--project-path".to_string(), project_path.clone()]);
            }
            if let Some(limit) = params.limit {
                args.extend(["--limit".to_string(), limit.to_string()]);
            }
            Ok(args)
        }
        "status" => {
            args.push("status".to_string());
            Ok(args)
        }
        "explain" => {
            let memory_id = params
                .memory_id
                .as_ref()
                .or(params.query.as_ref())
                .ok_or_else(|| "memory_id is required for omx-memory explain".to_string())?;
            args.extend(["explain".to_string(), memory_id.clone()]);
            Ok(args)
        }
        "note" => {
            let title = params
                .title
                .as_ref()
                .ok_or_else(|| "title is required for omx-memory note".to_string())?;
            let body = params
                .body
                .as_ref()
                .ok_or_else(|| "body is required for omx-memory note".to_string())?;
            args.extend(["note".to_string(), title.clone(), body.clone()]);
            if let Some(project_path) = &params.project_path {
                args.extend(["--project-path".to_string(), project_path.clone()]);
            }
            for tag in &params.tags {
                args.extend(["--tag".to_string(), tag.clone()]);
            }
            if let Some(importance) = params.importance {
                args.extend(["--importance".to_string(), importance.to_string()]);
            }
            Ok(args)
        }
        "ingest-transcript" | "ingest_transcript" => {
            let path = params
                .path
                .as_ref()
                .ok_or_else(|| "path is required for omx-memory ingest-transcript".to_string())?;
            let agent = params.agent.as_deref().unwrap_or("codex");
            args.push("ingest-transcript".to_string());
            args.extend(["--agent".to_string(), agent.to_string()]);
            if let Some(project_path) = &params.project_path {
                args.extend(["--project-path".to_string(), project_path.clone()]);
            }
            args.push(path.clone());
            Ok(args)
        }
        other => Err(format!("unsupported omx-memory action: {other}")),
    }
}

fn omx_memory_action_is_read_only(params: &CliToolInput) -> bool {
    matches!(
        params.action.as_deref().unwrap_or("retrieve"),
        "retrieve" | "status" | "explain"
    )
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

pub struct OmxMemoryTool;

#[async_trait]
impl Tool for OmxMemoryTool {
    fn name(&self) -> &str {
        claurst_core::constants::TOOL_NAME_OMX_MEMORY
    }

    fn description(&self) -> &str {
        "Use omx-memory for durable project/user memory. Actions: retrieve, status, explain, note, ingest-transcript. Prefer retrieve before answering when prior context may materially help."
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {"type": "string", "enum": ["retrieve", "status", "explain", "note", "ingest-transcript", "ingest_transcript"], "description": "omx-memory action; defaults to retrieve"},
                "query": {"type": "string", "description": "query for retrieve, or alias for memory_id in explain"},
                "memory_id": {"type": "string", "description": "memory ID for explain"},
                "title": {"type": "string", "description": "title for note"},
                "body": {"type": "string", "description": "body for note"},
                "tags": {"type": "array", "items": {"type": "string"}, "description": "tags for note"},
                "importance": {"type": "number", "description": "importance for note; default handled by omx-memory"},
                "path": {"type": "string", "description": "transcript path for ingest-transcript"},
                "agent": {"type": "string", "enum": ["codex", "claude"], "description": "transcript agent for ingest-transcript; defaults to codex"},
                "project_path": {"type": "string", "description": "project path filter/metadata for retrieve, note, or ingest-transcript"},
                "limit": {"type": "number", "description": "retrieve limit; default handled by omx-memory"},
                "db": {"type": "string", "description": "optional omx-memory database path; defaults to omx-memory CLI default"},
                "embedder": {"type": "string", "enum": ["hash", "model2-vec"], "description": "optional embedder provider"},
                "dimension": {"type": "number", "description": "hash embedder dimension"},
                "model2vec_model": {"type": "string", "description": "Model2Vec model ID or local model directory"},
                "args": {"type": "array", "items": {"type": "string"}, "description": "raw omx-memory arguments; overrides structured fields"},
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
        let is_read_only = omx_memory_action_is_read_only(&params);
        if let Err(e) = ctx.check_permission_for_path(
            self.name(),
            "Access durable memory with omx-memory",
            ctx.working_dir.clone(),
            is_read_only,
        ) {
            return ToolResult::error(e.to_string());
        }
        let args = match omx_memory_args(&params) {
            Ok(args) => args,
            Err(e) => return ToolResult::error(e),
        };
        run_cli(
            "omx-memory",
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
            db: None,
            embedder: None,
            dimension: None,
            model2vec_model: None,
            project_path: None,
            limit: None,
            title: None,
            body: None,
            tags: vec![],
            importance: None,
            memory_id: None,
            path: None,
            agent: None,
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
            db: None,
            embedder: None,
            dimension: None,
            model2vec_model: None,
            project_path: None,
            limit: None,
            title: None,
            body: None,
            tags: vec![],
            importance: None,
            memory_id: None,
            path: None,
            agent: None,
        };
        assert_eq!(
            graphifyq_args(&input).unwrap(),
            vec!["query", "how are tools wired?"]
        );
    }

    #[test]
    fn omx_memory_retrieve_args_include_global_and_project_options() {
        let input = CliToolInput {
            action: Some("retrieve".to_string()),
            query: Some("codex oauth".to_string()),
            question: None,
            queries: vec![],
            args: vec![],
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
            db: Some(".omx/test.sqlite".to_string()),
            embedder: Some("model2-vec".to_string()),
            dimension: None,
            model2vec_model: Some("minishlab/potion-base-8M".to_string()),
            project_path: Some("/workspace/claurst".to_string()),
            limit: Some(3),
            title: None,
            body: None,
            tags: vec![],
            importance: None,
            memory_id: None,
            path: None,
            agent: None,
        };

        assert_eq!(
            omx_memory_args(&input).unwrap(),
            vec![
                "--db",
                ".omx/test.sqlite",
                "--embedder",
                "model2-vec",
                "--model2vec-model",
                "minishlab/potion-base-8M",
                "retrieve",
                "codex oauth",
                "--project-path",
                "/workspace/claurst",
                "--limit",
                "3"
            ]
        );
        assert!(omx_memory_action_is_read_only(&input));
    }

    #[test]
    fn omx_memory_note_args_are_write_action() {
        let input = CliToolInput {
            action: Some("note".to_string()),
            query: None,
            question: None,
            queries: vec![],
            args: vec![],
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
            db: None,
            embedder: None,
            dimension: None,
            model2vec_model: None,
            project_path: Some("/workspace/claurst".to_string()),
            limit: None,
            title: Some("Decision".to_string()),
            body: Some("Use native OmxMemory tool.".to_string()),
            tags: vec!["claurst".to_string(), "memory".to_string()],
            importance: Some(0.9),
            memory_id: None,
            path: None,
            agent: None,
        };

        assert_eq!(
            omx_memory_args(&input).unwrap(),
            vec![
                "note",
                "Decision",
                "Use native OmxMemory tool.",
                "--project-path",
                "/workspace/claurst",
                "--tag",
                "claurst",
                "--tag",
                "memory",
                "--importance",
                "0.9"
            ]
        );
        assert!(!omx_memory_action_is_read_only(&input));
    }

    #[test]
    fn omx_memory_ingest_args_default_to_codex_agent() {
        let input = CliToolInput {
            action: Some("ingest_transcript".to_string()),
            query: None,
            question: None,
            queries: vec![],
            args: vec![],
            timeout_ms: DEFAULT_TIMEOUT_MS,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
            db: None,
            embedder: None,
            dimension: None,
            model2vec_model: None,
            project_path: None,
            limit: None,
            title: None,
            body: None,
            tags: vec![],
            importance: None,
            memory_id: None,
            path: Some("transcript.jsonl".to_string()),
            agent: None,
        };

        assert_eq!(
            omx_memory_args(&input).unwrap(),
            vec!["ingest-transcript", "--agent", "codex", "transcript.jsonl"]
        );
        assert!(!omx_memory_action_is_read_only(&input));
    }
}
