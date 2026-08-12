# Orchestrator Commands

Run these commands from the root of a project where an accelerator edition is
installed. They are available in Claude Code and Cursor. In Codex, use the
corresponding skills because Codex has no separate slash-command layer.

| Command | When to use it |
| --- | --- |
| `/flow-feature <request>` | Implement a complete feature: requirements → architecture → plan → approval → code → tests → parallel review → verification. |
| `/flow-review [scope]` | Review current changes in parallel for code quality, security, and performance. |
| `/sdd <feature>` | Run spec-driven development with durable artifacts in `specs/` and `tasks/`, approval checkpoints, and resumable execution. |

## Examples

- `/flow-feature Add two-factor authentication`
- `/flow-review src/Identity tests/Identity`
- `/sdd recurring-payments`

## Important

- The main AI session remains the orchestrator; subagents never spawn other
  agents.
- Execution stops at every checkpoint until the user explicitly approves it.
- Write-capable agents run sequentially. Only report-only review agents run in
  parallel.
- Use `/coder` for a small change and `/finishing-branch` after verification
  succeeds.

Full design: [Agent Orchestration Design](AGENT-ORCHESTRATION-DESIGN.md).
