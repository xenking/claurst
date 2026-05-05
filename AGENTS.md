# Claurst Agent Memory

_Last updated: 2026-05-06 (Australia/Melbourne)._  
_Canonical local working tree: `/Users/xenking/Projects/github.com/xenking/claurst`; Rust workspace lives in `src-rust/`._

## Repository workflow

- Inspect repo files with `fffq` first (`fffq ensure`, `fffq find`, `fffq grep`, `fffq multi-grep`), then semantic/graph tools.
- Use `graphifyq` for architecture/data-flow questions. Full repo graph is generated in `.graphify/` and can be queried with `graphifyq query "..."`.
- Build/test from `src-rust/`.
- Rust changes should be small and idiomatic; prefer focused tests and `cargo build -p claurst` before claiming done.
- Do not run broad `cargo fmt --all` casually: this repo has large existing formatting/CRLF drift and rustfmt wants to rewrite many unrelated files.
- `git diff --check` currently reports many CRLF-added lines as trailing whitespace; do not treat that alone as proof of broken code.
- Do not commit unless the user explicitly asks. When committing, use `xenking <support@xenking.pro>`.
- Keep untracked `.omx/cache`, `.omx/logs`, `.omx/metrics.json`, `.omx/state/*` out of commits unless specifically requested.

## Current fork state and recent commit

Recent local commit:

- `8c57692e4b5945ff91ccda46eefbc9da7c0fd4ca` — `fix codex ux and rtk controls`

That commit included:

- Codex OAuth UX fixes.
- `/fast` semantics corrected to be a transport/service-tier flag, not a model switch.
- `/rtk` native slash command added.
- TUI fixes for `Cmd+C`, paste/image paste, `/memory`, `/theme`, `/effort`, and `/settings` navigation.
- Image blocks are serialized to provider wire format as base64/url without leaking local paths.

## Codex provider notes

- User expects provider/model display like `openai-codex/gpt-5.5`.
- Codex OAuth provider runtime is implemented by `CodexProvider`, whose actual provider ID is `codex`; UI/config can use alias `openai-codex`.
- Provider lookup must account for both `openai-codex` and `codex`, especially in `/doctor`, command provider creation, and query dispatch.
- `/fast` must **never** change the selected model. It should preserve the current model and, for Codex OAuth requests, set provider option `serviceTier=priority`, mapped by the Codex adapter to request field `service_tier`.
- `/effort normal` should be accepted as alias for `medium`; valid levels are `low|medium|normal|high|max`.
- For Codex OAuth rate/limit display, prefer numeric usage/limits over dollar cost when possible.

## Important code areas

- CLI / TUI loop: `src-rust/crates/cli/src/main.rs`
- TUI state and key handling: `src-rust/crates/tui/src/app.rs`
- Slash commands: `src-rust/crates/commands/src/lib.rs`
- Provider registry and provider lookup: `src-rust/crates/api/src/registry.rs`
- Codex provider adapter: `src-rust/crates/api/src/providers/codex.rs`
- Model registry/context windows: `src-rust/crates/api/src/model_registry.rs`
- Core Codex model metadata: `src-rust/crates/core/src/codex_oauth.rs`
- Provider request path: `src-rust/crates/query/src/lib.rs`
- RTK native integration: `src-rust/crates/tools/src/rtk.rs`
- Settings UI: `src-rust/crates/tui/src/settings_screen.rs`
- Theme picker: `src-rust/crates/tui/src/theme_screen.rs`

## Recent bug findings and fixes

### Fast mode

Bug: `/fast` changed the active model to a mini/smaller model.  
Correct behavior: `/fast` spends the faster/higher-priority limit pool and keeps the selected model unchanged.

Implementation direction:

- UI persists `fast_mode` in `~/.claurst/ui-settings.json`.
- CLI loads/syncs `fast_mode` into `QueryConfig`.
- Query layer adds Codex provider option `serviceTier=priority` when fast mode is enabled.
- Codex provider maps `serviceTier` to JSON `service_tier`.

### Codex provider active status

Bug: `/doctor` reported `No active provider runtime found` for `openai-codex/gpt-5.5` after OAuth login.  
Fix: provider lookup aliases include both `openai-codex` and `codex`.

### Clipboard and paste

Bugs:

- `Cmd+C` from Claurst did not copy correctly.
- Image paste from clipboard could silently do nothing.
- Pasting big text could immediately submit on the trailing Enter.

Fixes:

- Handle macOS `SUPER+C` before keybinding processing; copy selected text or last assistant message and do not quit.
- Handle `Ctrl/Cmd+V` before keybinding processing; image first, text fallback.
- Guard Enter briefly after paste to avoid accidental submit.
- Big text/image paste should use blob/materialization paths rather than dumping huge content inline.

### Memory and theme dialogs

Bugs:

- `/memory` opened a dialog but `Enter`/`Space` did not open anything.
- `/theme` was not reliably applying choices.

Fixes:

- `/memory` selector now opens/creates the selected AGENTS.md via editor/open.
- `/theme <name>` is not intercepted by the picker; it goes through slash command executor.
- Theme picker accepts both `Enter` and `Space`.

### Settings navigation

Bug: arrow navigation in `/settings` did not reach several controls/tabs.  
Fix: settings screen now models selectable controls explicitly per tab and supports general/privacy/display navigation more predictably.

## RTK integration notes

Native RTK config exists in `Config.rtk` / `RtkConfig`:

- `enabled: bool`
- `mode: Off|Suggest|Rewrite`
- `binary: String` (default `rtk`)
- `exclude_commands: Vec<String>`
- `rewrite_timeout_ms: u64` (default 2000)

Slash command added:

- `/rtk status`
- `/rtk on|off|suggest|rewrite`
- `/rtk binary <path>`
- `/rtk timeout <ms>`
- `/rtk exclude add <prefix>`
- `/rtk exclude remove <prefix>`
- `/rtk exclude list`

RTK tool implementation is in `src-rust/crates/tools/src/rtk.rs`. It rewrites bash commands through the configured RTK binary and attaches rewrite metadata.

## Verification commands from the last session

Run from `src-rust/`:

```bash
cargo test -p claurst-tui test_argument_commands_are_not_intercepted --lib -- --test-threads=1
cargo test -p claurst-tui theme_space_applies_selected_theme --lib -- --test-threads=1
cargo test -p claurst-commands test_codex_provider_lookup_accepts_alias --lib -- --test-threads=1
cargo test -p claurst-commands test_rtk_status_command_returns_message --lib -- --test-threads=1
cargo test -p claurst-commands test_effort_command_accepts_medium_normal_and_max --lib -- --test-threads=1
cargo test -p claurst-query test_build_provider_options_for_openai_codex --lib -- --test-threads=1
cargo build -p claurst
```

These passed after commit `8c57692`.

## Graphify map

Full repo graph was regenerated on 2026-05-06:

```bash
graphify-rs build --path . --output .graphify --no-llm --embed --format json,report,context,wiki
graphifyq ensure
```

Current graph snapshot:

- `9678` nodes
- `17025` edges
- `585` communities
- Major entrypoints: `App`, `SlashCommand`, `PromptInputState`, `CopilotProvider`, `OpenAiProvider`, `OpenAiCompatProvider`, `render_message`, `apply_vim_key`.
- Highest-connectivity graph nodes: `10_utils`, `core::lib`, `09_bridge_cli_remote`, `01_core_entry_query`, `commands::lib`, `12_constants_types`, `04_components_core_messages`, `05_components_agents_permissions_design`, `prompt_input`, `02_commands`.

Use examples:

```bash
graphifyq query "how does codex oauth provider dispatch work?"
graphifyq query "where is fast mode wired from slash command to provider request?"
graphifyq query "how do clipboard paste and image paste flow through the TUI?"
```

## Next tasks / plan

1. **Run real interactive smoke test after user login**
   - Launch built `target/debug/claurst` in the fork.
   - Verify `/doctor` sees Codex OAuth as active.
   - Verify actual request with `openai-codex/gpt-5.5` and `/fast on` preserves model.

2. **Codex OAuth parity hardening**
   - Confirm native Codex request shape for `service_tier` / priority semantics against live behavior.
   - Add debug logging around final Codex request metadata without leaking tokens or prompt content.
   - Improve numeric rate-limit display for OAuth accounts.

3. **Clipboard/image paste E2E**
   - Add a real macOS clipboard image smoke path if feasible.
   - Verify pasted images become file/blob-backed `ContentBlock::Image` and then provider base64/url payloads.
   - Verify large text paste does not bloat prompt/context and never auto-submits.

4. **Settings/theme/memory UI polish**
   - Add E2E-ish TUI tests for `/settings` tab/arrow traversal.
   - Verify `/memory` opens the right file across User/Project/Local choices.
   - Ensure `/theme` persists and immediately applies across app refreshes.

5. **RTK integration battle test**
   - Install/configure `rtk` binary if absent.
   - Test `/rtk suggest`, `/rtk rewrite`, excludes, timeout behavior.
   - Run risky bash command examples through RTK and confirm metadata is visible.

6. **Native tool defaults**
   - Keep custom forked tools like `fffq` and `graphifyq` documented as preferred agent tools.
   - Consider exposing them as native/default tool hints in system prompt/config if Claurst supports it.

7. **Memory/long-term context layer**
   - Continue omx-memory/model2vec-rs integration separately.
   - Later wire finalized-turn memory retrieval into Claurst via MCP/native tools once stable.
