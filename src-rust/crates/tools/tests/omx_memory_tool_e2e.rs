use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::Arc;

use claurst_core::config::{Config, PermissionMode};
use claurst_core::permissions::AutoPermissionHandler;
use claurst_tools::{OmxMemoryTool, Tool, ToolContext};
use serde_json::json;

fn test_tool_context(working_dir: PathBuf) -> ToolContext {
    ToolContext {
        working_dir,
        permission_mode: PermissionMode::BypassPermissions,
        permission_handler: Arc::new(AutoPermissionHandler {
            mode: PermissionMode::BypassPermissions,
        }),
        cost_tracker: claurst_core::cost::CostTracker::new(),
        session_id: "omx-memory-e2e".to_string(),
        file_history: Arc::new(parking_lot::Mutex::new(
            claurst_core::file_history::FileHistory::new(),
        )),
        current_turn: Arc::new(AtomicUsize::new(0)),
        non_interactive: true,
        mcp_manager: None,
        config: Config::default(),
        managed_agent_config: None,
        completion_notifier: None,
        pending_permissions: None,
        permission_manager: None,
    }
}

#[tokio::test]
async fn omx_memory_tool_note_status_retrieve_round_trip() {
    if which::which("omx-memory").is_err() {
        eprintln!("skipping omx-memory e2e: omx-memory binary is not on PATH");
        return;
    }

    let temp_dir = tempfile::tempdir().expect("create temp dir");
    let db = temp_dir.path().join("memory.sqlite");
    let db = db.display().to_string();
    let project_path = temp_dir.path().join("project");
    let project_path = project_path.display().to_string();
    let ctx = test_tool_context(temp_dir.path().to_path_buf());
    let tool = OmxMemoryTool;

    let note = tool
        .execute(
            json!({
                "action": "note",
                "db": db,
                "title": "Claurst OmxMemory integration",
                "body": "Native OmxMemory tool can write and retrieve durable memory.",
                "project_path": project_path,
                "tags": ["claurst", "e2e"],
                "importance": 0.9
            }),
            &ctx,
        )
        .await;
    assert!(!note.is_error, "note failed: {}", note.content);
    assert!(note.content.contains("Claurst OmxMemory integration"));

    let status = tool
        .execute(json!({"action": "status", "db": db}), &ctx)
        .await;
    assert!(!status.is_error, "status failed: {}", status.content);
    assert!(status.content.contains("\"memories\": 1"));

    let retrieve = tool
        .execute(
            json!({
                "action": "retrieve",
                "db": db,
                "query": "native durable memory",
                "project_path": project_path,
                "limit": 3
            }),
            &ctx,
        )
        .await;
    assert!(!retrieve.is_error, "retrieve failed: {}", retrieve.content);
    assert!(retrieve.content.contains("Native OmxMemory tool"));
}
