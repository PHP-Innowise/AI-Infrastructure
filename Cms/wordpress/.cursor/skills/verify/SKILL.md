---
name: verify
description: Run the WordPress Definition of Done and report evidence-backed pass, fail, or N/A status before completion, PR, merge, or release.
phase: quality
flow-next: finishing-branch
flow-alternatives: [coder, systematic-debugger, test-generator]
related: [code-reviewer, security-reviewer, browser-verify]
---

# Verify WordPress Work

## Procedure

1. Select Minimum for docs/plans, Standard for implementation, or Full for
   merge/release. Read the active tool's `DOD.md` and project CI scripts.
2. Review `git diff --stat` and changed files. Confirm no secrets, generated
   local state, core files, unrelated vendor files, or accidental release
   artifacts changed.
3. Prefer configured Composer/npm/CI scripts. Where applicable run:
   Composer validation/audit, PHP syntax, PHPUnit/Pest/wp-browser, PHPCS with
   WordPress Coding Standards, PHPStan/Psalm, JavaScript unit/lint/style/build,
   and relevant WP-CLI, REST, block, multisite, WooCommerce or browser checks.
4. Verify behavior-specific controls: lifecycle, public compatibility, nonce
   plus capability, validation/escaping, SQL, migration rerun/rollback,
   cache invalidation, idempotent background work, accessibility and i18n.
5. Run focused tests while diagnosing, then the full relevant suite. Label a
   filtered run accurately. Never install missing tooling without approval;
   report `N/A - tooling not configured`.
6. Update verification authority only for Project Brain claims actually proven
   by this run, following the documented revision-checked lifecycle.

## Report

Provide tier; a table of check/status/evidence; overall PASS or FAIL; N/A
reasons; public/data/security/compatibility risks; exact failing commands and
short diagnostics; Context Summary; and next step. Any required failure means
FAIL—never soften it into a partial success.
