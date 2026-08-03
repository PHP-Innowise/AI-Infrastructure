---
name: verify
description: Run the Symfony Definition of Done and report pass/fail/N/A evidence before completion, PR, or merge.
phase: quality
flow-next: finishing-branch
flow-alternatives: [systematic-debugger, code-reviewer]
---

# Symfony Verify

## Workflow

1. Read the active edition's `DOD.md`.
2. Inspect changed files.
3. Select the correct DoD tier.
4. Run configured project commands first.
5. Run Symfony checks when relevant.
6. Promote Project Brain records this run confirmed.
7. Report pass/fail/N/A evidence.

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

## Promote Verified Brain Records

Verification is the only step that re-checks a claim against reality, so recording that outcome is part of the verification itself: a fact that passed but stays `observed` never qualifies for automatic promotion, and the pass would leave no trace in governed memory.

For every Project Brain record (finding, bug, incident, decision) whose claim this run actually confirmed, promote its authority BEFORE transitioning the record to a terminal status (`resolved`, `closed`, `accepted`):

```bash
python3 memory-bank/scripts/context.py brain-update \
  --record-id <id> --revision auto \
  --authority verified --reason "Verified: <check that confirmed it>"
```

Only the `observed -> verified` transition is accepted; the terminal transition follows in a separate `brain-update --transition <terminal-status>`. Skip records this run did not confirm — an unverified claim must stay `observed`.

## Output

Include:

- Tier used.
- Commands run and status.
- N/A tooling.
- Failures and next fix command.
- Final verdict.
