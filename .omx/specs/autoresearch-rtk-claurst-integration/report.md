# Native RTK integration research for Claurst

Date: 2026-05-05
RTK source snapshot: `/tmp/rtk-research` at `4338f02`
Claurst branch: `xenking/codex-lab`

## Executive verdict

Yes, RTK is worth integrating experimentally, but not by blindly vendoring it first.

The best first integration is a **native Claurst RTK adapter around Bash/PtyBash execution**:

1. detect external `rtk` binary;
2. ask `rtk rewrite <command>` for supported shell commands;
3. preserve Claurst's permission/security model by approving/classifying the original command before execution and logging both original and rewritten commands;
4. execute the rewritten command only when rewrite is safe and configured;
5. expose status/savings through `/rtk` or a small built-in `Rtk` tool.

This gives the biggest immediate gain (compact `cargo`, `git`, `gh`, test/build/lint/docker outputs) with the least maintenance risk.

Do **not** let RTK replace our FFF/Graphify/OmxMemory native surfaces. RTK is an output-compression proxy; Fffq and Graphifyq are code-intelligence/navigation primitives. For our workflow the priority stays:

- repository lookup: `Fffq` first;
- architecture/data-flow: `Graphifyq`;
- durable prior context: `OmxMemory`;
- noisy shell execution: RTK-assisted `Bash`/`PtyBash`.

## What RTK actually provides today

RTK is a Rust CLI proxy that runs known developer commands and filters their stdout/stderr before the output reaches an LLM context. Its README positions it as 60-90% token reduction on common dev commands.

Relevant implementation facts:

- `Cargo.toml` package: `rtk`, version `0.38.0`, Rust 2021.
- `LICENSE` file is Apache-2.0, while `Cargo.toml` says MIT and live GitHub metadata returned `license: null`. Treat license as unresolved before code vendoring.
- Command rewrite lives in `src/discover/registry.rs` and `src/discover/rules.rs`.
- `rtk rewrite <cmd>` delegates to `registry::rewrite_command` and returns an exit-code protocol in `src/hooks/rewrite_cmd.rs`:
  - `0`: rewrite allowed;
  - `1`: no RTK equivalent;
  - `2`: deny rule matched;
  - `3`: rewrite but ask/prompt.
- The rules cover many high-value commands: `git`, `gh`, `glab`, `cargo`, `go test/build/vet`, `golangci-lint`, `npm/pnpm/npx`, `jest/vitest/pytest`, `rg/grep/find/ls/cat/head/tail`, `docker`, `kubectl`, `curl`, etc.
- It has a tracking DB/config (`src/core/config.rs`) with `hooks.exclude_commands` and gain/history analytics.

## Agent support reality check

RTK's support quality differs per agent:

### Claude Code

Strongest integration. `hooks/claude/rtk-rewrite.sh` is a PreToolUse hook for Bash. It parses the tool input JSON with `jq`, calls `rtk rewrite`, and returns `updatedInput` so Claude Code runs the rewritten command. It also has version guards and permission-aware behavior.

### OpenCode

Programmatic hook. `hooks/opencode/rtk.ts` listens to `tool.execute.before`, checks Bash/shell tool calls, calls `rtk rewrite`, and mutates `args.command` in-place.

### Codex CLI

Prompt-only. `hooks/codex/README.md` explicitly says Codex support is prompt-level guidance only, not a programmatic hook. `rtk init --codex` writes `RTK.md` and injects an `@RTK.md` reference into `AGENTS.md`; it does not intercept tool execution.

### Kilo Code / Cline-like integrations

Prompt/rules-only in the inspected snapshot. Kilo Code installs `.kilocode/rules/rtk-rules.md` with instructions to prefix shell commands with `rtk`.

## Claurst integration surface

Claurst already has the right native place to integrate RTK:

- Built-in tools are registered in `src-rust/crates/tools/src/lib.rs::all_tools()`.
- `PtyBashTool` is currently the primary Bash tool in `all_tools()`.
- Bash execution paths:
  - `src-rust/crates/tools/src/bash.rs::BashTool::execute`
  - `src-rust/crates/tools/src/pty_bash.rs::PtyBashTool::execute`
- Both tools:
  - parse `command` input;
  - run `ctx.check_permission_for_path(...)`;
  - block `BashRiskLevel::Critical` via `classify_bash_command`;
  - build a wrapper script preserving cwd/env;
  - execute via `bash -c`/PTY and return captured output.
- Existing native inspection tools already exist in `cli_inspection.rs`: `FffqTool`, `GraphifyqTool`, `OmxMemoryTool`.
- System prompt already tells the model to prefer Fffq before Grep/Glob/Bash and Graphifyq after Fffq (`core/src/system_prompt.rs`).

Important limitation: Claurst's current generic `PreToolUse` hook path is not enough for RTK-style rewriting. `run_hooks` can return `HookOutcome::Modified(stdout)`, but `query/src/lib.rs` only uses PreToolUse to block/allow; it does not apply modified tool input before `execute_tool`. So simply installing RTK as a Claurst hook would not transparently rewrite Bash commands today.

## Recommended design

### Phase 1: external-binary native adapter

Add a small `rtk` module inside `claurst-tools` or `claurst-core`:

- Config:
  - `rtk.enabled: bool` default false (or auto only in YOLO/lab build);
  - `rtk.mode: off | suggest | rewrite`;
  - `rtk.binary: Option<PathBuf>` default `rtk`;
  - `rtk.exclude_commands: Vec<String>`;
  - maybe `rtk.prefer_for_file_lookup: false` to preserve FFF-first.
- Detection:
  - `which rtk` / `rtk --version` with min version guard (`>=0.23.0` because RTK hook docs require `rewrite`).
- Rewrite helper:
  - call `rtk rewrite <original_command>` using argv, not shell string interpolation;
  - interpret RTK exit codes `0/1/2/3`;
  - on error/missing RTK: pass through original command and log one warning.
- Bash/PtyBash integration:
  - permission prompt/classifier should consider the original command and display both original and rewritten command when rewrite happens;
  - never rewrite `Critical` commands;
  - rewrite only foreground and background Bash commands after command parsing, before wrapper-script construction;
  - store metadata in `ToolResult.metadata` or debug logs: original, rewritten, rtk exit code.
- UI/commands:
  - `/rtk status` shows enabled/mode/binary/version;
  - `/rtk on|off` toggles session config;
  - `/rtk gain` calls `rtk gain` if installed.

This is low-risk because RTK remains an optional external dependency. Claurst can still build/run without RTK installed.

### Phase 2: generic modified PreToolUse support

Separately fix Claurst's hook executor so `HookOutcome::Modified` can update `PreparedTool.input`. Then a Claude-Code-style RTK hook could work too. This benefits other hook use cases, not only RTK.

Caution: this needs tests because modified tool input changes permissions/security semantics. The safest policy is: run permission evaluation on the original input and re-run risk classification after modification for Bash.

### Phase 3: selective vendoring/forking only if needed

Only consider vendoring/cherry-picking RTK internals after the external-binary adapter proves useful. Reasons to defer vendoring:

- RTK's crate is a binary, not a clean library API for embedding.
- The repo has a license inconsistency in the inspected snapshot (`LICENSE` Apache-2.0, `Cargo.toml` MIT, GitHub metadata null).
- It has many dependencies and command-specific behavior we may not want to maintain inside Claurst.
- External subprocess `rtk rewrite` is already the single source of truth used by Claude/OpenCode integrations.

If we eventually fork, the useful pieces to cherry-pick are mostly:

- `src/discover/lexer.rs`
- `src/discover/registry.rs`
- `src/discover/rules.rs`
- tests around compound command rewriting, redirects, `RTK_DISABLED`, heredocs, and `gh --json` skip behavior.

## Expected benefits for our workflow

High-value wins:

- less transcript bloat from `cargo test`, `cargo build`, `cargo clippy`, `gh run/checks`, `git diff/status/log`, docker/kubectl/log commands;
- cheaper/faster multi-agent loops when executors run noisy commands;
- better failure extraction from test/build output;
- optional savings analytics via `rtk gain`.

Medium/low-value areas:

- file search/read commands overlap with `Fffq`, `Read`, `Grep`, `Glob`; do not route these to RTK by default in Claurst system prompt.
- RTK is less useful for native MCP/tool calls and structured API tools because it only filters shell command output.

## Risks and mitigations

| Risk | Mitigation |
|---|---|
| Agent stops using Fffq/Graphifyq and uses `rtk grep/find` for repo inspection | Keep system prompt priority and tool descriptions explicit: Fffq first, Graphifyq for architecture, RTK only for Bash output compression. |
| Permission bypass through command rewrite | Check permission/risk on original command, show both commands, and re-classify rewritten Bash before execution. Do not auto-allow because RTK rewrote. |
| Hidden output causes debugging misses | Add `RTK_DISABLED=1` / config exclude / `/rtk off`, and log original+rewritten command. |
| Missing `rtk` binary breaks shell | Adapter must pass through original command when RTK is absent or too old. |
| License/maintenance risk if vendored | Do not vendor in phase 1. Resolve Apache/MIT mismatch first. |
| PTY behavior differs | Integrate at command-string level before wrapper construction; keep PTY runner unchanged. |

## Concrete implementation plan

1. Add `RtkConfig` to core config/settings.
2. Add `rtk_rewrite` helper with unit tests for missing binary, exit-code mapping, no-rewrite, rewrite, ask, deny.
3. Integrate helper into `PtyBashTool::execute` and `BashTool::execute` before wrapper creation and background spawn.
4. Add debug logs and metadata for original/rewrite decisions.
5. Add `/rtk status|on|off|gain` command or `RtkTool`.
6. Add system-prompt note: RTK may compress Bash output, but Fffq/Graphifyq remain preferred for repo inspection.
7. E2E battle tests:
   - `git status` rewritten to `rtk git status` when enabled and RTK present;
   - `gh pr view --json ...` is not rewritten;
   - `rm -rf /` remains blocked;
   - missing RTK passes through;
   - background Bash path rewrites safely;
   - PTY Bash still preserves cwd/env;
   - agent transcript shows compact command output and logs enough to debug.

## Bottom line

Proceed with RTK as an optional native Bash accelerator for Claurst. The experimental value is high for agent efficiency/performance, especially in YOLO-style workflows, but the first iteration should be an external-binary adapter rather than a vendored fork. This keeps Claurst stable while letting us measure whether RTK materially improves task completion speed and context efficiency.
