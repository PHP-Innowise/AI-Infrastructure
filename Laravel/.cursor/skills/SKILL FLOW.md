# Skill Flow

This flow keeps Laravel work structured while preserving user control. Agents suggest the next command but do not automatically chain.

## Main Flow

```text
/requirements-analyst
  -> /researcher        (when options/libraries/approaches are unclear)
  -> /brainstorm
  -> /council           (for high-stakes trade-offs)
  -> /architect
  -> /database-designer (when the data model is non-trivial)
  -> /api-designer
  -> /frontend-design
  -> /writing-plans
  -> /git-worktrees
  -> /architecture-implementer   (scaffold the decided structure)
  -> /coder or /coder-frontend or /filament (admin panel work)
       or a specialized implementation skill: /eloquent, /queues-jobs,
       /events-notifications, /auth-scaffolding, /caching,
       /console-scheduler, /file-storage, /package-developer
  -> /code-reviewer
  -> /security-reviewer (for security-sensitive changes)
  -> /test-generator
  -> /performance-optimization   (when speed/resource use matters)
  -> /verify
  -> /finishing-branch
```

## Shortcuts

- Use `/coder` directly for small, well-understood Laravel fixes.
- Use `/researcher` before `/council` or `/architect` when you need sourced evidence.
- Use `/council` when a decision has real, competing trade-offs.
- Use `/architecture-implementer` to turn an `/architect` decision into a compiling skeleton before `/coder`.
- Use `/database-designer` before `/coder` when schema, keys, or indexing are unclear.
- Use `/api-designer` before `/coder` when route, request, response, or error contracts are unclear.
- Use `/filament` for admin-panel/internal-tooling CRUD screens; use `/coder-frontend` for customer-facing UI even on the same project.
- Use `/eloquent` for model-layer behavior beyond basic CRUD: polymorphic relationships, custom casts, query scopes, model events/Observers, or large-dataset iteration.
- Use `/queues-jobs` for any asynchronous/background work: job classes, job middleware, unique jobs, batching/chaining, or Horizon configuration.
- Use `/events-notifications` for decoupled side effects (Events/Listeners, Observers) and multi-channel user communication (Notifications, Mailables).
- Use `/auth-scaffolding` for web/session auth starter kits (Breeze/Jetstream/Fortify), multi-guard setup, and Policy/Gate authorization; use `/api-designer` instead for token-based API auth (Sanctum/Passport/JWT).
- Use `/caching` when a read is expensive/repeated, or as the implementation of a fix identified by `/performance-optimization`.
- Use `/console-scheduler` for custom Artisan commands and recurring/scheduled tasks.
- Use `/file-storage` for any feature that stores, serves, or accepts user-uploaded files.
- Use `/package-developer` only when the deliverable is a standalone Composer package, not an application feature.
- Use `/test-generator` after `/coder` when coverage is missing.
- Use `/refactorer` for behavior-preserving cleanup under a test safety net.
- Use `/security-reviewer` for auth, input-handling, SQL, upload, or secret-touching changes.
- Use `/performance-optimization` when something is measurably slow.
- Use `/dependency-manager` for Composer audits, updates, and vetting new packages.
- Use `/debugger` when tests fail for unclear reasons or behavior is unexpected.
- Use `/docs-generator` when setup, deployment, worker/cron, API, or architecture documentation changed.
- Use `/project-brain` for governed task lifecycle, handoffs, unified retrieval, all six governed record types, compaction, and automatic or independently reviewed promotions. Governed mode is the default; `--mode lightweight` is an explicit local-only fallback.
- Use `/memory-bank` only for durable retrieval/capture/audit/supersession and governed automatic or independently reviewed promotion application; active work stays in Project Brain.
- Use `/checkpoint`, `/memory` for authority-aware progress capture and unified context refresh; governed mode never creates SQLite task authority.

## Flows (opt-in orchestration)

A flow command runs several roster agents from the main conversation in one
run: sequential stages, parallel only for read-only agents, and a mandatory
pause at every declared checkpoint. Flows are the one sanctioned exception to
"agents do not automatically chain" (see AGENTS.md, Orchestration section)
and cost several agent runs' worth of tokens - use them for well-understood
multi-phase work, not for small fixes.

- `/flow-feature` - requirements -> architecture -> plan -> **checkpoint**
  -> code -> tests -> parallel review (code + security) -> verify ->
  **checkpoint** -> finishing-branch.
- `/flow-review` - code-reviewer, security-reviewer and
  performance-optimization in parallel (read-only), then one deduplicated
  report synthesized in the main conversation.
- `/sdd` - spec-driven development: specify -> design -> **checkpoint** ->
  task breakdown -> **checkpoint** -> execute task by task -> tests ->
  parallel review -> verify. Every phase leaves a durable artifact in
  `specs/` and `tasks/`, so the run can be resumed in a later session; see
  the `sdd` skill for the artifact contract. Does not integrate - finish
  with `/finishing-branch`.

Each stage hands the next agent a bounded Task Capsule (see below); the user
can amend, skip a stage, or abort at any checkpoint.

## Phase Map

| Phase | Commands |
| --- | --- |
| Understanding | `/requirements-analyst`, `/codebase-mapper`, `/researcher`, `/brainstorm` |
| Planning | `/council`, `/architect`, `/database-designer`, `/api-designer`, `/frontend-design`, `/writing-plans` |
| Implementation | `/git-worktrees`, `/architecture-implementer`, `/coder`, `/coder-frontend`, `/filament`, `/eloquent`, `/queues-jobs`, `/events-notifications`, `/auth-scaffolding`, `/caching`, `/console-scheduler`, `/file-storage`, `/package-developer`, `/refactorer` |
| Quality | `/code-reviewer`, `/security-reviewer`, `/test-generator`, `/performance-optimization`, `/debugger`, `/verify` |
| Finalization | `/docs-generator`, `/release`, `/finishing-branch` |
| Utility | `/project-brain`, `/checkpoint`, `/memory`, `/memory-bank`, `/reflect`, `/skill-creator`, `/review-pr`, `/browser-verify`, `/dependency-manager` |

## Task Capsule Handoff

At a complex phase boundary, the orchestrating agent builds one bounded Task
Capsule from a concise sanitized retrieval query, optional Working state, and
layered context. A fresh phase agent receives the capsule and explicit
current-step files, not the parent conversation.

The returning handoff contains only:

- work completed;
- decisions made;
- files changed or examined;
- verification evidence;
- risks and assumptions;
- the next step or recommended next command;
- unresolved blockers or questions;
- memory chunk IDs used or changed, when applicable;
- Project Brain task/record revisions, handoff, and retrieval manifest, when
  applicable;
- cited authoritative sources.

The next agent must not preload every cited source. It opens one only when the
current step requires more information. Simple tasks remain in the current
context.
