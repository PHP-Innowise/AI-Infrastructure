---
name: using-git-worktrees
description: Create and use isolated git worktrees for Symfony implementation tasks.
phase: planning
flow-next: coder
flow-alternatives: [coder-frontend, test-generator]
related: [writing-plans, coder]
---

# Using Git Worktrees

## Overview

Use git worktrees to isolate feature work, experiments, or parallel implementations without disturbing the main working directory.

## PHP Considerations

- Run `composer install` in the worktree if `vendor/` is not shared or present.
- Do not copy `.env` from another worktree automatically.
- Use a separate local database or SQLite test database if migrations/tests would conflict.
- Run migrations only against the intended local database.
- Run focused tests before returning work to the main branch.

## Safe Workflow

```bash
git worktree add ../project-feature feature/project-feature
cd ../project-feature
composer install
composer test
```

If frontend tooling exists:

```bash
<frontend-install-command>
<frontend-build-command>
```

## Branch And Commit Hygiene

- Name branches by intent: `feature/...`, `fix/...`, `refactor/...`, `chore/...`.
- Make small, focused commits; keep refactors separate from behavior changes so review and rollback stay clean.
- Write imperative commit subjects ("Add invitation expiry check"); explain the "why" in the body when non-obvious.
- Rebase or merge from the base branch regularly to avoid a large, conflict-heavy merge at the end.
- Never commit `vendor/`, `.env`, secrets, or build artifacts; verify with `git status` before committing.

## Completion

Before removing a worktree:

- Commit or intentionally discard work.
- Confirm no untracked files are needed.
- Remove the worktree with `git worktree remove <path>`.

## Final Output

Return worktree path, branch name, setup commands run, Context Summary, and next command.
