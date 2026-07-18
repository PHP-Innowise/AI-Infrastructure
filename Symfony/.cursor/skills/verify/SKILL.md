---
name: verify
description: Run the Symfony Definition of Done and report pass/fail/N/A evidence before completion, PR, or merge.
phase: quality
flow-next: finishing-branch
flow-alternatives: [debugger, code-reviewer]
---

# Symfony Verify

## Workflow

1. Read `.cursor/DOD.md`.
2. Inspect changed files.
3. Select the correct DoD tier.
4. Run configured project commands first.
5. Run Symfony checks when relevant.
6. Report pass/fail/N/A evidence.

## Common Commands

```bash
composer validate --strict
composer audit
composer test
composer lint
composer analyse
php bin/console lint:container
php bin/console debug:router
php bin/console doctrine:schema:validate --skip-sync
```

Use direct `vendor/bin/*` equivalents when Composer scripts do not exist.

## Output

Include:

- Tier used.
- Commands run and status.
- N/A tooling.
- Failures and next fix command.
- Final verdict.
