# Autoresearch mission: native RTK integration for Claurst

Date: 2026-05-05
Validation mode: `prompt-architect-artifact`

## Question
Can Claurst integrate `https://github.com/rtk-ai/rtk` natively, and is it worth doing for our agent workflow?

## Scope
- Inspect RTK's current repo structure and its support for Codex, Claude Code, OpenCode, Kilo Code, and related agents.
- Inspect Claurst's internal tool execution path to find a safe integration point.
- Decide whether the integration should be prompt-only, hook-based, external-binary adapter, or vendored Rust code.
- Preserve our existing native tools priority: Fffq first, Graphifyq for architecture, OmxMemory for durable memory.

## Validation criteria
- Evidence-backed findings from the RTK repo and the Claurst repo.
- Clear recommendation with risks, non-goals, and a staged implementation plan.
- Completion artifact includes architect verdict and output artifact path.
