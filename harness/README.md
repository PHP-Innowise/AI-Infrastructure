# AI-Infrastructure Harness

External LangGraph batch harness for the AI-Infrastructure accelerators:
unattended multi-stage pipelines (nightly fleet review, mass migrations,
scheduled research) over headless coding-agent hosts.

This is **Stage D** of the agent-orchestration design
([`../docs/AGENT-ORCHESTRATION-DESIGN.md`](../docs/AGENT-ORCHESTRATION-DESIGN.md))
and a deliberate **companion, not a component**: it lives at the monorepo
root, OUTSIDE the editions — the installer never ships it, the inventories
never list it, no edition requires or depends on it, and the accelerator's
own runtime stays stdlib-only. This directory (with its own venv) is where
the LangGraph dependency is allowed to live.

## What it does

```
harness run --project /path/to/project --scope "HEAD~5..HEAD"
# ... runs lenses in parallel, then:
# PAUSED at the approval gate: {"findings": 7, "high": 2, "cost_usd": 1.84}
harness resume --project /path/to/project --thread fleet-review-... --approve
```

The `fleet-review` graph: scope → parallel review lenses (`Send` fan-out)
→ collect → **human approval gate** (`interrupt`, waits indefinitely across
processes) → report + record. Durable execution via the SQLite checkpointer:
a crashed or paused run resumes from its checkpoint with `--thread`.

## Architecture rules (from the design doc)

- **The blackboard is the project's own project-brain.** Nodes write and
  read through the target project's `context.py`: task lifecycle, capsule
  validation (`capsule --validate` refuses under-specified delegations),
  the message channel (`msg-dispatch` spawn/complete with SHA-256 capsule
  digests). LangGraph's checkpointer holds only graph position — interactive
  sessions and unattended runs share one audit trail.
- **Workers are headless host sessions.** The default `claude-cli` worker
  runs `claude -p --output-format json` in the project directory — the
  officially documented subprocess pattern — deliberately **without**
  `--bare`, so the project's `.claude/` world applies inside every worker:
  permissions deny rules, the subagent gate (roster-only spawning, write
  serialization), skills, and the SubagentStop observer. The worker prompt
  delegates to the project's own roster agent. `codex-cli` (`codex exec
  --json`, read-only sandbox) is available; Cursor has no adapter on
  purpose — its headless mode has documented hangs.
- **Cost is a first-class signal.** `total_cost_usd` from each worker
  accumulates in graph state; `--budget-usd` (default $5) is a hard ceiling
  — lenses beyond it are skipped and reported as skipped, never silently
  dropped. Expect multi-agent runs to cost 3–15× a single-agent session.
- **`dry-run` worker** rehearses any pipeline offline (deterministic canned
  findings, no host, no tokens) — used by the test suite and recommended
  before every new pipeline.

## Install

```bash
python3 -m venv .venv && .venv/bin/pip install -e ".[dev]"
.venv/bin/python -m pytest            # 8 tests, offline
```

Requires Python ≥ 3.10. The target project needs the accelerator installed
(for the blackboard); without `memory-bank/scripts/context.py` the harness
still runs, minus the audit trail.

## Auth for unattended runs

Subscription OAuth does not survive CI: use `ANTHROPIC_API_KEY`, or mint a
long-lived token with `claude setup-token`. Note Anthropic's policy: products
must not offer claude.ai login or rate limits — internal pipelines on your
own subscription are fine. Parallel workers share the subscription's rate
bucket; throttle accordingly (`--worker-timeout`, fewer lenses).

## When NOT to use this

Interactive work belongs in the accelerator's own `/flow-*` commands (Stage
A–C) — the main conversation is the orchestrator there, with checkpoints the
user answers in place. Short single-host scripts are simpler with the Claude
Agent SDK alone. This harness earns its dependencies only for long,
resumable, multi-stage unattended runs with human gates.
