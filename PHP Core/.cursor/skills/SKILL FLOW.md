# Skill Flow

This flow keeps native PHP work structured while preserving user control. Agents suggest the next command but do not automatically chain.

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
  -> /coder or /coder-frontend
  -> /code-reviewer
  -> /security-reviewer (for security-sensitive changes)
  -> /test-generator
  -> /performance-optimization   (when speed/resource use matters)
  -> /verify
  -> /finishing-branch
```

## Shortcuts

- Use `/coder` directly for small, well-understood native PHP fixes.
- Use `/researcher` before `/council` or `/architect` when you need sourced evidence.
- Use `/council` when a decision has real, competing trade-offs.
- Use `/architecture-implementer` to turn an `/architect` decision into a compiling skeleton before `/coder`.
- Use `/database-designer` before `/coder` when schema, keys, or indexing are unclear.
- Use `/api-designer` before `/coder` when route, request, response, or error contracts are unclear.
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
| Implementation | `/git-worktrees`, `/architecture-implementer`, `/coder`, `/coder-frontend`, `/refactorer` |
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
