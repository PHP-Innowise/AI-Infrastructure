---
name: using-git-worktrees
description: Create and use isolated git worktrees for Laravel/PHP implementation tasks.
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
- Do not copy `.env` from another worktree automatically.
- Use a separate local database or SQLite test database if migrations/tests would conflict.
- Recreate storage links with `php artisan storage:link` if needed.
- Run migrations only against the intended local database.
- Run focused tests before returning work to the main branch.

## Safe Workflow

```bash
git worktree add ../project-feature feature/project-feature
cd ../project-feature
composer install
php artisan test
```

If frontend tooling exists:

```bash
<frontend-install-command>
<frontend-build-command>
```

## Completion

Before removing a worktree:

- Commit or intentionally discard work.
- Confirm no untracked files are needed.
- Remove the worktree with `git worktree remove <path>`.

## Final Output

Return worktree path, branch name, setup commands run, Context Summary, and next command.
