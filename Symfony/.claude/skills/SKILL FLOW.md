# Symfony Layered Architecture Skill Flow

This flow keeps Symfony work structured while preserving user control. Agents suggest the next command but do not automatically chain.

All implementation, review, and planning work must respect root `AGENTS.md`, `specs/MANIFEST.md`, and `examples/symfony-clean-code-patterns.md`.

## Main Flow

```text
/requirements-analyst
  -> /researcher                       (when options/libraries/approaches are unclear)
  -> /brainstorm
  -> /council                          (for high-stakes trade-offs)
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
  -> /security-reviewer                (for security-sensitive changes)
  -> /test-generator
  -> /performance-optimization         (when speed/resource use matters)
  -> /verify
  -> /finishing-branch
```

## Symfony Shortcuts

- Use `/coder` directly for small, well-understood Symfony Controller -> Service -> Repository fixes.
- Use `/researcher` before `/council` or `/architect` when you need sourced evidence about Symfony components, bundles, Doctrine, API Platform, Messenger, or security approaches.
- Use `/architecture-implementer` to turn an `/architect` decision into a compiling Symfony skeleton before `/coder`.
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
- Use `/memory-bank` only to retrieve, capture, audit, supersede, archive, or initialize durable source-backed project memory; keep transient progress in task context.

## Phase Map

| Phase | Commands |
| --- | --- |
| Understanding | `/requirements-analyst`, `/researcher`, `/brainstorm` |
| Planning | `/council`, `/architect`, `/database-designer`, `/doctrine-migration-designer`, `/api-designer`, `/api-platform-designer`, `/frontend-design`, `/writing-plans`, `/security-voter-designer`, `/form-validator-designer`, `/messenger-designer`, `/event-subscriber-designer` |
| Implementation | `/git-worktrees`, `/architecture-implementer`, `/coder`, `/coder-frontend`, `/console-command-coder`, `/fixture-factory-generator`, `/refactorer` |
| Quality | `/architecture-boundary-reviewer`, `/code-reviewer`, `/repository-reviewer`, `/security-reviewer`, `/twig-ux-reviewer`, `/container-reviewer`, `/test-generator`, `/performance-optimization`, `/debugger`, `/verify` |
| Finalization | `/docs-generator`, `/release`, `/finishing-branch` |
| Utility | `/memory-bank`, `/reflect`, `/skill-creator`, `/review-pr`, `/browser-verify`, `/dependency-manager` |

## Context Handoff

Every skill must finish with:

- What changed or was decided.
- Controller/service/repository placement when implementation is involved.
- Files/specs touched.
- Verification evidence or planned verification.
- Risks and assumptions.
- Memory chunk IDs used or changed, when applicable.
- Recommended next command.
