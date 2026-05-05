# Sandbox notes

## Sources inspected

### RTK
- Live GitHub metadata via `gh repo view rtk-ai/rtk` on 2026-05-05.
- Shallow clone at `/tmp/rtk-research`, commit `4338f02`.
- FFF initialized in `/tmp/rtk-research` with `fffq ensure`.
- Key files:
  - `README.md`
  - `Cargo.toml`
  - `LICENSE`
  - `CHANGELOG.md`
  - `hooks/codex/README.md`
  - `hooks/codex/rtk-awareness.md`
  - `hooks/claude/README.md`
  - `hooks/claude/rtk-rewrite.sh`
  - `hooks/opencode/README.md`
  - `hooks/opencode/rtk.ts`
  - `hooks/kilocode/README.md`
  - `hooks/kilocode/rules.md`
  - `src/hooks/init.rs`
  - `src/hooks/rewrite_cmd.rs`
  - `src/discover/registry.rs`
  - `src/discover/rules.rs`
  - `src/core/config.rs`
  - `src/cmds/README.md`

### Claurst
- Repo: `/Users/xenking/Projects/github.com/xenking/claurst`, branch `xenking/codex-lab`.
- FFF initialized with `fffq ensure`.
- Graphifyq architecture query run for Bash/PtyBash integration surface.
- Key files:
  - `src-rust/crates/tools/src/lib.rs`
  - `src-rust/crates/tools/src/bash.rs`
  - `src-rust/crates/tools/src/pty_bash.rs`
  - `src-rust/crates/tools/src/cli_inspection.rs`
  - `src-rust/crates/core/src/bash_classifier.rs`
  - `src-rust/crates/core/src/system_prompt.rs`
  - `src-rust/crates/core/src/lib.rs`
  - `src-rust/crates/query/src/lib.rs`
  - `docs/hooks.md`

## Environment notes
- `rtk` is not currently installed in PATH on this machine (`command -v rtk` returned no output).
- RTK repository metadata from GitHub returned license `null`, while `Cargo.toml` says MIT and `LICENSE` is Apache-2.0; this must be clarified before vendoring code.
- Existing Claurst branch had runtime `.omx/` artifacts already untracked. Only this research directory should be staged if committed.
