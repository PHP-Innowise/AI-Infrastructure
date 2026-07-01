# Skill Flow

This flow keeps Laravel/PHP work structured while preserving user control. Agents suggest the next command but do not automatically chain.

## Main Flow

```text
/requirements-analyst
  -> /brainstorm
  -> /architect
  -> /api-designer
  -> /frontend-design
  -> /writing-plans
  -> /git-worktrees
  -> /coder or /coder-frontend
  -> /code-reviewer
  -> /test-generator
  -> /verify
  -> /finishing-branch
```

## Laravel Shortcuts

- Use `/coder` directly for small, well-understood Laravel fixes.
- Use `/api-designer` before `/coder` when route, request, resource, or error contracts are unclear.
- Use `/test-generator` after `/coder` when coverage is missing.
- Use `/debugger` when tests fail for unclear reasons or behavior is unexpected.
- Use `/docs-generator` when setup, deployment, queue, schedule, API, or architecture documentation changed.

## Phase Map

| Phase | Commands |
| --- | --- |
| Understanding | `/requirements-analyst`, `/brainstorm` |
| Planning | `/architect`, `/api-designer`, `/frontend-design`, `/writing-plans` |
| Implementation | `/git-worktrees`, `/coder`, `/coder-frontend` |
| Quality | `/code-reviewer`, `/test-generator`, `/debugger`, `/verify` |
| Finalization | `/docs-generator`, `/release`, `/finishing-branch` |
| Utility | `/reflect`, `/skill-creator`, `/review-pr`, `/browser-verify` |

## Context Handoff

Every skill should finish with:

- What changed or was decided.
- Files/specs touched.
- Verification evidence or planned verification.
- Risks and assumptions.
- Recommended next command.
