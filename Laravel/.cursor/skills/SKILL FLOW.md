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
- Use `/memory-bank` only to retrieve, capture, audit, supersede, archive, or initialize durable source-backed project memory; keep transient progress in task context.

## Phase Map

| Phase | Commands |
| --- | --- |
| Understanding | `/requirements-analyst`, `/researcher`, `/brainstorm` |
| Planning | `/council`, `/architect`, `/database-designer`, `/api-designer`, `/frontend-design`, `/writing-plans` |
| Implementation | `/git-worktrees`, `/architecture-implementer`, `/coder`, `/coder-frontend`, `/filament`, `/eloquent`, `/queues-jobs`, `/events-notifications`, `/auth-scaffolding`, `/caching`, `/console-scheduler`, `/file-storage`, `/package-developer`, `/refactorer` |
| Quality | `/code-reviewer`, `/security-reviewer`, `/test-generator`, `/performance-optimization`, `/debugger`, `/verify` |
| Finalization | `/docs-generator`, `/release`, `/finishing-branch` |
| Utility | `/memory-bank`, `/reflect`, `/skill-creator`, `/review-pr`, `/browser-verify`, `/dependency-manager` |

## Context Handoff

Every skill should finish with:

- What changed or was decided.
- Files/specs touched.
- Verification evidence or planned verification.
- Risks and assumptions.
- Recommended next command.
- Memory chunk IDs used or changed, when applicable.
