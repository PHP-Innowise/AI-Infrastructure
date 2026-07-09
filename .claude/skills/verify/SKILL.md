---
name: verify
description: Run the native PHP Definition of Done and report pass/fail status. Use before claiming completion, creating a PR, or merging.
phase: execution
flow-next: finishing-branch
flow-alternatives: [coder, debugger, test-generator]
related: [code-reviewer, test-generator]
---

# Verify

## Overview

Run `.claude/DOD.md` and produce a clear pass/fail report. Do not install missing tools. Report missing tooling as `N/A - tooling not configured`. Prefer the project's Composer scripts when they exist.

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

Run when files/tooling exist (prefer Composer scripts, fall back to `vendor/bin/*`):

```bash
composer validate --strict
php -l <changed-files>
composer test        # or vendor/bin/phpunit / vendor/bin/pest
composer lint        # or vendor/bin/php-cs-fixer fix --dry-run --diff / vendor/bin/phpcs
composer analyse     # or vendor/bin/phpstan analyse / vendor/bin/psalm
```

Pick the commands that match the project. Do not run both Pest and PHPUnit if the project standardizes on one.

If server-rendered frontend files changed and tooling exists:

```bash
<html-or-template-lint-command>
<css-lint-command>
<frontend-build-command>
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
- Migration, worker/queue, and cache impacts are documented.

## Report Template

```markdown
## Verification Report

**Tier:** Standard
**Date:** YYYY-MM-DD

| Check | Status | Evidence |
| --- | --- | --- |
| Working tree | PASS | `git diff --stat` reviewed |
| Composer | PASS | `composer validate --strict` |
| Tests | PASS | `composer test` |
| Formatting | N/A | php-cs-fixer not configured |
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
