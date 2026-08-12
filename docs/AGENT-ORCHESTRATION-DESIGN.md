> Дизайн: слой оркестрации и коммуникации агентов поверх существующего
> Command → Agent → Skill и project-brain. Сводка по-русски — в ответе
> сессии; ниже полный документ. Статус: Stage A реализована (2026-08-08:
> `/flow-feature`, `/flow-review`, секция Orchestration в AGENTS.md, Flows в
> SKILL FLOW.md, потолки agents_md подняты по политике бюджета). Stage B
> реализована (2026-08-08: канал `msg-send`/`msg-read`/`msg-dispatch`,
> `capsule --validate`, `update --actor`, гард фазовых переходов,
> `message.schema.json`, секция Messages в PROTOCOL.md; отличие от
> первоначального наброска: канал не индексируется в retrieval — оркестратор
> читает его явно через `msg-read`, бюджеты retrieval не тронуты). Stage C
> реализована (2026-08-08: хук-наблюдатель `subagent-dispatch.sh` на
> SubagentStop/subagentStop, `writes: true` во фронтматтере + сериализация
> пишущих агентов TTL-замком в гейтах, `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1`;
> решение по Codex принято — multi-agent остаётся выключенным). Stage D
> реализована (2026-08-08) как проект-компаньон: LangGraph-граф
> fleet-review (Send-фан-аут линз -> interrupt-гейт -> отчёт),
> SQLite-чекпоинтер, воркеры `claude -p`/`codex exec`/`dry-run`,
> blackboard — через `context.py` целевого проекта. Живёт в каталоге
> `harness/` в корне монорепо — вне редакций, инсталлятора и инвентарей,
> со своим venv (MCP-shape: опционально, редакции от него не зависят);
> см. раздел в TOOL-INTEGRATIONS.md.

# Agent orchestration & communication layer — historical proposal and status

## Current implementation status

Stages A-D are implemented. Stages A-C are part of the maintained accelerator:
flow commands, the governed message channel and capsule validation, phase/actor
guards, the SubagentStop observer, and write-agent serialization. Stage D is
the optional repo-root `harness/` companion and is not shipped in editions or
installer inventories. The remainder of this document preserves the design
reasoning recorded before and during implementation; statements labeled as
gaps or proposed work describe that historical point, not missing current
functionality.

## Historical proposal

**Question.** Can the accelerator gain an orchestration/communication layer so
that multi-step work (feature flow, review flow, research fan-out) runs as a
coordinated set of roster agents instead of the user typing every next
command — without breaking the isolation policy, the subagent gate, or the
three-tool parity model?

**Verdict at proposal time.** Yes. The repository already had four
proto-orchestration layers (a declarative flow graph in command frontmatter,
SKILL FLOW's phase map, the Task Capsule handoff discipline, and the governed
project-brain state store). The proposed executor, message channel with
per-agent identity, and completion signaling were subsequently implemented in
Stages A-C on the existing markdown, hook, and stdlib CLI substrate.

Everything below rests on facts verified this session (2026-08-08) against
official docs of the three hosts and the repo itself; the research transcripts
carry the citations.

---

## 1. Baseline before implementation

| Layer | What it was | Gap at proposal time |
|---|---|---|
| Flow graph | Every spawning Claude command carried `spawns` / `phase` / `flow-next` / `flow-alternatives` frontmatter — a machine-readable suggested-next-command graph | There was no executor: all 31 spawning commands were single-agent wrappers and the user was the scheduler. Fan-out, join, and condition vocabulary had not been added. |
| Phase map | `SKILL FLOW.md` Main Flow / Phase Map | It was presentation-only and transitions were user-driven. Phase names differed from PROTOCOL.md's five-value enum (an alias table existed in `brain_runtime.normalize_phase`). |
| Handoff discipline | Task Capsule (8,000 chars, 2 procedural / 3 semantic / 1 episodic) + one schema-validated, CAS-guarded handoff file per task | The baseline had one rewritten handoff per task, no mailbox or per-agent identity, poll-only completion, and no parallel-writer awareness beyond stale-revision errors. |
| State store | project-brain: locked, revisioned records; retrieval manifests already gave who-read-what-when auditing | It was state-based rather than message-based; two agents in one task communicated by rewriting and reading the same handoff. |

Precedent inside the repo: Infrastructure-Creator's **Orchestration
Exception** already lets five named skills fan out (infra-scan runs seven
scanners in parallel where the host supports it). The PHP editions have no
such exception yet — adding one is a policy edit, not an invention.

## 2. Verified host capabilities (2026-08)

| Capability | Claude Code | Cursor | Codex |
|---|---|---|---|
| Nested spawning | up to 3 levels below main (default; `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH`) | main + direct children may spawn; grandchildren may not | depth 1 by default; deeper is undocumented |
| Parallel fan-out | yes; 20 concurrent default (`CLAUDE_CODE_MAX_CONCURRENT_SUBAGENTS`) | yes (multiple Task calls); `is_background: true` for non-blocking | yes, when multi-agent is enabled (currently **off** in this repo) |
| Inter-agent messaging | `SendMessage` between named agents in-session; resume-by-ID keeps history | none — parent conversation, filesystem (`~/.cursor/subagents/`), or followup loops | richest: v2 `send_message`, `followup_task`, `wait_agent` (mailbox), `interrupt_agent`, `list_agents` |
| Output capture for hooks | `SubagentStop` → `last_assistant_message` + `agent_transcript_path` — reliable capture point | `subagentStop.summary` / `modified_files` **broken in practice** (staff-confirmed bug; никогда не срабатывает for background) — do not build on it | `SubagentStop` → `last_assistant_message` + transcript — workable |
| Per-agent spawn allowlists | only for main-thread `claude --agent` (`Agent(name)` in `tools`); ignored in subagent definitions → **our PreToolUse gate remains the only roster enforcement at depth** | `subagentStart` deny gate (already shipped) | PreToolUse on `spawn_agent`; roster filter keys on `tool_input.agent_type`, which is only exposed when `.codex/agents/*.toml` roster is non-empty; role spawns must use `fork_turns: "none"`; hooks are explicitly *not* a hard boundary |

Two structural consequences:

- **The orchestrator must be the main conversation**, not an "orchestrator
  subagent". The main loop has no depth ceiling above it, sees every result,
  holds the permission mode, and is the only place `Agent(...)` restrictions
  and human checkpoints are fully honored. Nesting exists but adds cost and
  removes oversight; cap it instead (`CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1`).
- **The three tools cannot share one orchestration engine**, only one
  substrate. Files + the context.py CLI are the common denominator; host
  primitives (SendMessage, `is_background`, Codex collab tools) are
  progressive enhancement per edition, exactly like the existing
  command/agent-layer asymmetry (Codex already has no agents by design).

## 3. Design principles (from measured prior art)

1. **Orchestrator-workers, hyper-specific delegation.** Every dispatch carries
   four mandatory fields — objective, output format, tool/source guidance,
   task boundaries (Anthropic's research-system findings) — plus a
   **decisions-and-assumptions** section (Cognition's failure mode: parallel
   workers fabricate divergent assumptions). The capsule schema enforces this.
2. **Blackboard over message-routing.** Content never travels through the
   orchestrator's context; capsules carry *paths* into the git-tracked store
   (pass-by-reference artifacts). The project-brain already is the blackboard;
   the main conversation plays the control-unit role explicitly.
3. **Parallel only where it pays.** Fan-out is empirically strong for
   breadth-first read-heavy work (research/scan/review; Anthropic: +90.2% on
   their eval) and weak for tightly-coupled edits. Policy: read-only roles may
   run in parallel; write-capable roles are serialized by the orchestrator.
4. **Single-writer-per-file.** The substrate has no concurrency control beyond
   CAS; convention (each parallel worker owns its output path; the
   orchestrator merges) plus a hook check closes the gap.
5. **No graph engine.** Resumability and partial-run value already come from
   the task lifecycle in the git store. What LangGraph would add that matters
   — allowed status transitions — is a small table the CLI can enforce.
6. **Opt-in and priced.** Multi-agent flows cost 3–15× tokens. Manual flow
   stays the default; flows are explicit commands whose descriptions state the
   multiplier.

## 4. Architecture — three stages

### Stage A — flow commands (no new machinery)

A new command class, `/flow-*`, whose body instructs the **main conversation**
to act as orchestrator: resolve the flow, spawn roster agents stage by stage
via the Agent/Task tool, pass a delegation capsule in each prompt, collect
results, write the handoff between phases, pause at human checkpoints.

- Flow definition: extend the existing frontmatter vocabulary rather than
  invent a format. `spawns`/`flow-next`/`flow-alternatives` stay the
  sequential baseline; a flow command adds a `stages:` list — each stage =
  agents (roster names), `parallel: true|false`, `checkpoint: true|false`.
  Phase values use PROTOCOL.md's five-value enum (`normalize_phase` reuse).
- Two initial flows:
  - `/flow-feature`: requirements-analyst → architect → writing-plans →
    **checkpoint (human approves plan)** → coder → test-generator →
    [code-reviewer ∥ security-reviewer] → verify → finishing-branch.
  - `/flow-review`: [code-reviewer ∥ security-reviewer ∥
    performance-optimization (read-only mode)] → synthesis in main.
- Policy edit: AGENTS.md gains an **Orchestration Exception** mirroring
  Infrastructure-Creator's — "MUST NOT chain" becomes "MUST NOT chain
  *except inside a sanctioned `/flow-*` command, which chains only roster
  agents and pauses at declared checkpoints".
- Tool parity: Claude gets the full flow; Cursor gets the same commands
  (its subagents support parallel Task calls); Codex keeps sequential
  skill-flow prose (no agents there by design — unchanged).

Stage A needs zero Python changes and works today: the capsule travels inside
the spawn prompt; results return as each agent's final message plus its
`update` to the task handoff.

### Stage B — communication substrate (context.py extensions)

This stage added the then-missing primitives on the existing machinery (schema
validation, CAS, lock, atomic writes, and retrieval budgets were already built):

1. **`context.py msg`** — append-only, task-scoped mailbox
   (`send --task-id ID --from coder --to code-reviewer --type
   finding|question|handoff --payload-file p.md`, `read --for NAME [--new]`).
   Message bodies obey capsule sanitization and size bounds at the transport;
   content by reference where possible. Storage: one JSONL per task under
   project-brain local state, indexed into retrieval like handoffs.
2. **Per-agent identity** — an `actor` field on messages and updates (roster
   name), distinct from the installation-level `owner`.
3. **Dispatch log** — reuse the retrieval-manifest pattern: one record per
   spawn (who, whom, capsule digest, task revision) and per completion. This
   supplied the observability layer and completion signal absent in the
   proposal baseline.
4. **Delegation-capsule schema** — the four mandatory fields + assumptions
   section, validated by the CLI (`context.py capsule --validate`), so a flow
   cannot dispatch an under-specified task.
5. **Allowed-transitions table** — status/phase transitions by role, enforced
   at `update`/`complete` (the cheap LangGraph substitute).

Cross-edition rules apply: byte-identical Python across the three PHP
editions, tests in `test_hooks.py`/`test_context*`, root CHANGELOG entry.

### Stage C — host-native enhancement (per edition)

- **Claude Code**: SubagentStop hook (`subagent-dispatch.sh`) appends
  `agent_type` + `last_assistant_message` to the dispatch log automatically;
  SendMessage enabled between named roster agents for clarifying questions
  (reviewer ↔ coder) instead of respawn round-trips; resume-by-ID for
  continuing an agent. Env knobs in settings.json:
  `CLAUDE_CODE_MAX_SUBAGENT_SPAWN_DEPTH=1` (orchestrator lives in main; no
  nested trees), keep the concurrent default.
- **Cursor**: parallel Task calls + `is_background` for long read-only
  stages; capture via transcript files, **not** `subagentStop.summary` (bug).
- **Codex** (decision point): either keep multi-agent off — flows remain
  sequential prose, current posture — or re-enable selectively: create
  `.codex/agents/*.toml` roster (this is also what makes `agent_type` appear
  in the spawn schema), flip `[agents] enabled = true`, and switch
  `subagent-gate.sh` from deny-all to roster-allowlist on
  `tool_input.agent_type` (deny spawns that omit it; instruct
  `fork_turns: "none"` in AGENTS.md). Codex's v2 collab tools
  (`send_message`/`wait_agent` mailbox) are the richest of the three once
  enabled — but hooks there are "guardrail, not boundary", so the config
  switches stay the hard story.

## 5. Guardrails

- **Roster-only at any depth** — already enforced by the shipped
  `subagent-gate.sh` on all three tools (PreToolUse fires for nested spawns
  too). Flows add no new spawn authority.
- **Write serialization** — extend the Claude gate: while a write-capable
  agent (coder, refactorer, architecture-implementer, ...) is active (lock
  file keyed by task), deny spawning a second write-capable agent; read-only
  roles (researcher, codebase-mapper, code-reviewer, security-reviewer,
  scanners) pass. Roster metadata gains a `writes: true|false` frontmatter
  key — the transplanted "Agent Card".
- **Context-overlap decomposition** — flows sequence agents that must read
  the same files (coder → test-generator) with an explicit capsule; only
  disjoint-context roles pair in parallel stages.
- **Human checkpoints** — every flow declares them; plan approval before
  execution is mandatory in `/flow-feature`. Autonomous end-to-end runs are
  possible but never the default.
- **Effort scaling** — orchestrator instructions cap stage width by task
  complexity; the concurrent-limit env var is the hard backstop.

## 6. What we deliberately do NOT build

- **An orchestrator subagent** — depth limits, no per-type spawn allowlists
  in subagent definitions, invisible to the user; the main loop is strictly
  better.
- **A DAG/graph engine** — the git store already gives resume and partial
  value; a transitions table suffices.
- **A2A/ACP/MCP-as-bus** — network protocols for cross-machine interop;
  nothing here leaves the repo. Two ideas transplant wire-free: Agent Cards
  (roster capability metadata) and a typed task-state lifecycle.
- **Agent Teams / background sessions as the base** — experimental or
  session-external; revisit when stable if cross-session parallelism is ever
  needed.

## 7. Compatibility with the subagent gate (2026-08-08)

The gate allowlists roster names — orchestration happens strictly inside the
roster, so nothing shipped yesterday blocks Stages A/B. Two touchpoints:
Stage C-Codex replaces deny-all with roster-allowlist (config switches
`[agents] enabled` / `[features] multi_agent` flip to true in that scenario);
the write-serialization guardrail extends the same gate script rather than
adding a new hook.

## 8. Implemented checklist (repo conventions)

1. AGENTS.md carries the Orchestration Exception.
2. `/flow-feature` and `/flow-review` exist for Claude and Cursor; Codex keeps
   sequential skills.
3. Stage B ships `msg-send`, `msg-read`, `msg-dispatch`, capsule validation,
   actor-aware updates, phase guards, schema, parity checks, and tests.
4. Stage C ships the SubagentStop observer and write-capable-agent
   serialization; Codex multi-agent remains disabled by decision.
5. Settings, generated mirrors/inventories, flow catalogs, and release records
   were updated with the implementation.
6. Infrastructure-Creator propagation of the full hand-built flow/channel
   surface remains separate follow-up work; the base subagent gate is already
   generated.

## 9. LangChain / LangGraph evaluation (2026-08-08)

Asked explicitly: can LangChain and/or LangGraph implement the orchestration
and communication layers? Verified against PyPI/GitHub/official docs, a real
`pip install` in a scratch venv, and this repo's own documents.

**The technology itself is sound.** LangGraph 1.2.x is MIT, ~10 months into a
kept no-breaking-changes-until-2.0 pledge; nodes are arbitrary Python
functions (no model API keys required — "they can contain an LLM or just good
ol' code"), so *node = subprocess to a headless coding agent* is a normal
pattern. It ships exactly the primitives Stages A/B hand-build: durable
execution + SQLite checkpointer, `interrupt()`/`Command(resume=)` human
checkpoints, `Send` fan-out, per-node `RetryPolicy`, typed shared-state
channels, time-travel replay. Measured cost: 39 packages / ~68 MB / Python
≥3.10. The outer-harness pattern is real: Anthropic officially sanctions
driving `claude -p --output-format json` as a subprocess; `codex exec --json`
is first-class; Cursor's headless `agent -p` exists but is operationally
flaky (staff-confirmed hangs). LangChain proper adds nothing on top — its
1.x agents *are* LangGraph; classic chains moved to `langchain-classic`.

**As the accelerator's core layer — no.** Five documented collisions:

1. README_EN/RU name LangGraph a non-feature verbatim ("It does not provide
   embeddings, vector search, MCP, LangGraph, a central memory service…");
   the deferral is recorded at spec level in two superpowers docs.
2. The runtime contract is stdlib-only / copy-only installer / no packaging
   manifest anywhere / Python 3.9 floor (langgraph needs ≥3.10) / PHP-team
   audience that manages composer.json, not venvs — and the contract already
   survived a measured challenge (embeddings) and was kept deliberately.
3. No-daemon invariant (`docs/OPERATIONS.md`), metadata-only hooks, and the
   degrade-to-noop guarantee when python3 is absent.
4. An outer LangGraph process is a fourth executor that holds no host
   permission mode and sits outside the interactive session — the human
   checkpoints and roster gate live inside the hosts (§2 of this doc).
5. House precedent: TOKEN-ECONOMY-RESEARCH's verdict ("integrate no
   third-party tool…"; ccusage rejected for "adding a Node prerequisite to a
   stdlib house style") is the evaluation frame this proposal would face.
   A local LangGraph prototype (`langgraph_app`, editable install still
   visible in the ignored `.venv/`) was already built beside this repo and
   never admitted into the tree.

**Where it had real prospects: an external, opt-in batch harness (the "MCP
shape": optional, separate, never required).** For unattended flows —
nightly fleet review, mass migrations, scheduled research — where durable
resume, retries and `interrupt()` gates genuinely pay and no interactive
session exists:

- Lives outside editions in its own venv and never enters inventories, mirrors,
  or the installer. The implemented companion is the repo-root `harness/`
  project; editions do not provide or depend on it.
- Nodes drive hosts headlessly: Claude Agent SDK (preferred for Python — by
  default it loads the project's `.claude/` world, so settings.json deny
  rules and `subagent-gate.sh` still apply *inside* every worker) or
  `claude -p` / `codex exec --json`; Cursor node optional behind hard
  timeouts. `total_cost_usd` from JSON output feeds cost attribution.
- Communication stays canonical: node code writes/reads through
  `context.py` (task state, capsules, Stage B mailbox) — LangGraph's
  checkpointer holds only graph position, so interactive sessions and the
  harness see the same blackboard and the audit trail stays in one place.
- Auth: subscription OAuth breaks in CI — API keys or `claude setup-token`;
  bare mode never reads OAuth credentials. Budget for the 3–15× multiplier.
- Decision rule vs plain Agent SDK: single-host, short pipelines → SDK
  alone (fewer moving parts); long resumable multi-stage or multi-host
  flows with human gates → LangGraph earns its 39 packages.

**Implemented verdict.** Stages A-C remain in-session and stdlib-based.
LangGraph was rejected as the core layer and used only for the optional
**Stage D external batch harness** now present under repo-root `harness/`.
LangChain as such remains not applicable.

## 10. Open decisions

1. **Codex posture** — *resolved 2026-08-08 with Stage C*: multi-agent stays
   off. The edition delegates through skills by design, hooks there are "a
   guardrail, not a boundary", and the roster surface a selective allowlist
   would require does not exist. Revisit only if a real Codex multi-agent
   need appears; the selective-gate recipe stays documented in §4 Stage C.
2. **Mailbox storage** — *resolved with Stage B*: the message channel is
   Git-tracked under Project Brain control state and validated as an
   append-only sequence; it is not ignored local JSONL.
3. **Flow definition location** — *resolved with Stage A*: flow commands carry
   their orchestration contract directly; no separate flow manifest was added.
