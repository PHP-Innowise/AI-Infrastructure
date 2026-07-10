---
name: verify
description: Run the Laravel Definition of Done and report pass/fail status. Use before claiming completion, creating a PR, or merging.
phase: execution
flow-next: finishing-branch
flow-alternatives: [coder, debugger, test-generator]
related: [code-reviewer, test-generator]
---

# Verify

## Overview

Run `.codex/DOD.md` and produce a clear pass/fail report. Do not install missing tools. Report missing tooling as `N/A - tooling not configured`. Prefer the project's Composer/Artisan scripts when they exist. This branch targets Laravel; for framework-agnostic native PHP, use the `main` branch.

## Step 1: Determine Tier

- Documentation/planning only: Minimum.
- Implementation task: Standard.
- PR, merge, or release readiness: Full.

## Step 2: Run Applicable Checks

### Minimum

```bash
git diff --stat
```

Also check:

- Skill-prefixed files in `tasks/` and `specs/`.
- Context Summary and Next Steps.
- No secrets or `.env` access.

### Standard

Run when files/tooling exist (prefer Composer/Artisan scripts, fall back to `vendor/bin/*`):

```bash
composer validate --strict
php -l <changed-files>
php artisan test          # or vendor/bin/pest / vendor/bin/phpunit
vendor/bin/pint --test    # Laravel Pint formatting check
vendor/bin/phpstan analyse  # Larastan config, or vendor/bin/psalm
```

Pick the commands that match the project. Do not run both Pest and PHPUnit as separate suites if the project standardizes on one (Pest can run PHPUnit-style test classes too). For larger suites, prefer `php artisan test --parallel` when the project already has `brianium/paratest` configured; do not force it onto a project that doesn't use it.

If server-rendered Blade/frontend files changed and tooling exists:

```bash
<blade-or-template-lint-command>
<css-lint-command>
<frontend-build-command>   # e.g. npm run build (Vite)
```

### Full

```bash
composer audit
gh run list --limit 1
```

Also verify:

- PR description has summary and test plan.
- Changelog/specs/docs updated when required.
- No unresolved TODO/FIXME/HACK in changed source files.
- New/changed migrations are reversible and reviewed for production data impact.
- Queue/Horizon, cache, and config-cache impacts are documented.

## Report Template

```markdown
## Verification Report

**Tier:** Standard
**Date:** YYYY-MM-DD

| Check | Status | Evidence |
| --- | --- | --- |
| Working tree | PASS | `git diff --stat` reviewed |
| Composer | PASS | `composer validate --strict` |
| Tests | PASS | `php artisan test` |
| Formatting | PASS | `vendor/bin/pint --test` |
| Static analysis | PASS | `vendor/bin/phpstan analyse` (Larastan) |

**Result:** PASS

### Notes
- No unresolved risks.

### Context Summary
[2-3 sentences]

### Next Steps
- `/finishing-branch` if ready.
```

## Failure Handling

If any required check fails:

- Mark result as FAIL.
- Include the exact command.
- Include the shortest useful failure summary.
- Recommend `/coder` for clear fixes or `/debugger` for unclear failures.

## Final Output

Return the verification report and stop.
