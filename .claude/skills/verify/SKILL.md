---
name: verify
description: Run the Laravel/PHP Definition of Done and report pass/fail status. Use before claiming completion, creating a PR, or merging.
phase: execution
flow-next: finishing-branch
flow-alternatives: [coder, debugger, test-generator]
related: [code-reviewer, test-generator]
---

# Verify

## Overview

Run `.claude/DOD.md` and produce a clear pass/fail report. Do not install missing tools. Report missing tooling as `N/A - tooling not configured`.

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

Run when files/tooling exist:

```bash
composer validate
php artisan test
vendor/bin/pest
vendor/bin/phpunit
vendor/bin/pint --test
vendor/bin/phpstan analyse
vendor/bin/psalm
php artisan route:list
```

Pick the commands that match the project. For example, do not run both Pest and PHPUnit if the project clearly standardizes on one command.

If frontend tooling exists and frontend files changed:

```bash
<frontend-lint-command>
<frontend-build-command>
```

### Full

```bash
gh run list --limit 1
```

Also verify:

- PR description has summary and test plan.
- Changelog/specs/docs updated when required.
- No unresolved TODO/FIXME/HACK in changed source files.
- Migration, queue, schedule, and cache impacts are documented.

## Report Template

```markdown
## Verification Report

**Tier:** Standard
**Date:** YYYY-MM-DD

| Check | Status | Evidence |
| --- | --- | --- |
| Working tree | PASS | `git diff --stat` reviewed |
| Composer | PASS | `composer validate` |
| Tests | PASS | `php artisan test` |
| Formatting | N/A | Pint not configured |
| Static analysis | PASS | `vendor/bin/phpstan analyse` |

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
