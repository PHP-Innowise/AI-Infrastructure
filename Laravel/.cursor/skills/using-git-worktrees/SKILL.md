---
name: using-git-worktrees
description: Create and use isolated git worktrees for Laravel implementation tasks.
phase: planning
flow-next: coder
flow-alternatives: [coder-frontend, test-generator]
related: [writing-plans, coder]
---

# Using Git Worktrees

## Overview

Use git worktrees to isolate feature work, experiments, or parallel implementations without disturbing the main working directory.

## Laravel Considerations

- Run `composer install` in the worktree if `vendor/` is not shared or present.
- Copy or symlink `.env` from the main worktree rather than committing it; never copy it automatically without confirming secrets/DB credentials are appropriate for the new worktree.
- Run `php artisan key:generate` if the copied `.env` has no `APP_KEY` or a fresh one is needed.
- Use a separate local database (or SQLite in-memory/file database for tests) so migrations don't conflict with other worktrees.
- Run `php artisan migrate` (or `migrate:fresh` for a clean local DB) against the intended local database only.
- Run `php artisan test` before returning work to the main branch.

## Safe Workflow

```bash
git worktree add ../project-feature feature/project-feature
cd ../project-feature
composer install
cp ../project/.env .env          # or symlink; adjust DB_DATABASE per worktree
php artisan key:generate          # only if APP_KEY is missing
php artisan migrate               # or migrate:fresh for a clean local DB
php artisan test
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
