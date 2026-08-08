# Symfony Layered Architecture Skill Flow

This flow keeps Symfony work structured while preserving user control. Agents suggest the next command but do not automatically chain.

All implementation, review, and planning work must respect root `AGENTS.md`, `specs/MANIFEST.md`, and `examples/symfony-clean-code-patterns.md`.

## Main Flow

```text
/requirements-analyst
  -> /researcher        (when options/libraries/approaches are unclear)
  -> /brainstorm
  -> /council           (for high-stakes trade-offs)
  -> /architect
  -> /database-designer                (when Doctrine schema/data model is non-trivial)
  -> /doctrine-migration-designer      (when migration/rollout risk exists)
  -> /api-designer
  -> /api-platform-designer            (only when API Platform is used)
  -> /frontend-design
  -> /writing-plans
  -> /git-worktrees
  -> /architecture-implementer         (scaffold controller-service-repository skeleton)
  -> /coder or /coder-frontend
  -> /architecture-boundary-reviewer   (for layer-sensitive changes)
  -> /code-reviewer
  -> /repository-reviewer              (for Doctrine-heavy changes)
  -> /security-reviewer (for security-sensitive changes)
  -> /test-generator
  -> /performance-optimization   (when speed/resource use matters)
  -> /verify
  -> /finishing-branch
```

## Symfony Shortcuts

- Use `/coder` directly for small, well-understood Symfony Controller -> Service -> Repository fixes.
- Use `/researcher` before `/council` or `/architect` when you need sourced evidence about Symfony components, bundles, Doctrine, API Platform, Messenger, or security approaches.
- Use `/architecture-implementer` to turn an `/architect` decision into a compiling skeleton before `/coder`.
- Use `/database-designer` before `/coder` when entities, relationships, keys, indexes, constraints, or migrations are unclear.
- Use `/doctrine-migration-designer` before `/coder` when schema changes need safe rollout/backfill planning.
- Use `/api-designer` before `/coder` when route, request, response, serializer, status-code, or error contracts are unclear.
- Use `/api-platform-designer` only when the project uses API Platform.
- Use `/security-voter-designer` before `/coder` for object-level authorization.
- Use `/form-validator-designer` before `/coder` for complex Forms/request DTOs/custom constraints.
- Use `/messenger-designer` before `/coder` for async/retryable workflows.
- Use `/event-subscriber-designer` before `/coder` when listeners/subscribers are considered.
- Use `/console-command-coder` for Symfony console commands that delegate to services.
- Use `/twig-ux-reviewer` for Twig/Symfony UX frontend changes.
- Use `/container-reviewer` when DI/autowiring/tags/decorators/env config changed.
- Use `/fixture-factory-generator` when tests need realistic deterministic data.
- Use `/refactorer` for behavior-preserving cleanup under a test safety net.
- Use `/security-reviewer` for auth, voters, access control, input handling, Doctrine query, upload, SSRF, session, CSRF, serializer, or secret-touching changes.
- Use `/performance-optimization` when something is measurably slow, especially Doctrine N+1, Twig rendering, cache, Messenger workers, or memory pressure.
- Use `/dependency-manager` for Composer audits, Symfony bundle vetting, Symfony Flex recipe impact, and dependency updates.
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
| Planning | `/council`, `/architect`, `/database-designer`, `/doctrine-migration-designer`, `/api-designer`, `/api-platform-designer`, `/frontend-design`, `/writing-plans`, `/security-voter-designer`, `/form-validator-designer`, `/messenger-designer`, `/event-subscriber-designer` |
| Implementation | `/git-worktrees`, `/architecture-implementer`, `/coder`, `/coder-frontend`, `/console-command-coder`, `/fixture-factory-generator`, `/refactorer` |
| Quality | `/architecture-boundary-reviewer`, `/code-reviewer`, `/repository-reviewer`, `/security-reviewer`, `/twig-ux-reviewer`, `/container-reviewer`, `/test-generator`, `/performance-optimization`, `/debugger`, `/verify` |
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
- controller/service/repository placement when implementation is involved;
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
