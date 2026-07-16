---
name: console-command-coder
description: Implement Symfony console commands that validate input, delegate to services, stream progress safely, and return correct exit codes.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, verify]
---

# Symfony Console Command Coder

Rules:

- Command parses arguments/options and validates them.
- Business workflow lives in a service.
- Output is clear, non-secret, and script-friendly.
- Large work is batched/streamed and memory-safe.
- Exit codes are correct.
- Tests use `CommandTester` or project convention.

## Implementation Workflow

1. Inspect existing command naming, attributes, service boundaries, logging, and test conventions.
2. Define arguments/options, defaults, interactive behavior, exit codes, and machine-readable output expectations.
3. Validate and normalize input before invoking the service. Use Symfony Console helpers only for presentation and interaction.
4. Delegate one application workflow to a service; keep Doctrine queries and transaction logic out of the command.
5. Batch large datasets with repository iterators/keyset pagination, clear managed entities when appropriate, and make restart/idempotency behavior explicit.
6. Send diagnostics to stderr, avoid secrets and personal data, and preserve stable stdout for automation.
7. Handle expected domain/input failures with concise messages and non-zero exit codes; let unexpected failures remain observable.
8. Test success, invalid input, service failure, non-interactive mode, and exit codes with `CommandTester`.

Compare command/service separation with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). A command is a framework adapter, not a second implementation of the use case.

Do not add `--force` as a substitute for authorization or operational safeguards. Destructive commands require explicit confirmation, environment protection, and documented recovery.
