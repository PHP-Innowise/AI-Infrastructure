---
name: wp-cli
description: Build safe WP-CLI commands for WordPress operations, migrations, imports, maintenance, diagnostics, and batch processing with idempotency, progress, exit codes, multisite scope, and destructive guards.
phase: execution
flow-next: verify
flow-alternatives: [test-generator, code-reviewer, systematic-debugger]
related: [plugin-development, database-designer, multisite]
---

# WP-CLI

## Procedure

1. Confirm WP-CLI is available only in CLI context and register after the
   owning plugin is loaded. Follow existing command namespaces and bootstrap.
2. Define synopsis, required/optional positional and associative arguments,
   defaults, allowed values, examples, output format, and exit semantics.
3. Validate before mutation. Destructive commands require explicit scope,
   confirmation unless `--yes`, a dry-run when feasible, and a backup/rollback
   note. Never infer a production target from ambiguous input.
4. Make long operations restartable and idempotent. Batch by stable cursor,
   avoid loading all IDs, report progress without excessive logs, and record
   enough checkpoint state to resume safely.
5. State multisite semantics. Iterate sites only when requested, restore the
   original blog after every switch, and isolate failures so one site cannot
   silently corrupt the run.
6. Use `WP_CLI::success`, `warning`, `error`, `log`, and machine-readable output
   consistently. Do not print secrets or personal data.
7. Reuse application services; the command adapts arguments/output and must not
   become the only implementation of business behavior.
8. Test parsing, dry-run, idempotent rerun, partial failure/resume, empty data,
   invalid scope, permissions/operational guardrails, and exit status.

## Output

Return command synopsis/examples, mutation and recovery contract, batching and
multisite behavior, test evidence, Context Summary, and next step.
