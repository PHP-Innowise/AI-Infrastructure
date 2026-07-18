---
name: architecture-boundary-reviewer
description: Review Symfony Controller -> Service -> Repository boundaries and SOLID violations.
phase: quality
flow-next: code-reviewer
flow-alternatives: [refactorer, coder, verify]
---

# Symfony Architecture Boundary Reviewer

## Review Workflow

1. Read every changed entry point and trace each call through validation, authorization, application service, repository/external boundary, response mapping, and side effects.
2. Draw the actual dependency direction. Framework and infrastructure code may depend on application behavior; application behavior must not depend on controllers, Twig, Console, Messenger handlers, or service-container lookup.
3. Identify the owner of each decision: input mapping, authorization, workflow, invariant, transaction, query shape, external effect, and presentation.
4. Distinguish a real boundary violation from a small cohesive operation. Do not demand a pass-through layer or interface that adds no decision, contract, or substitution value.
5. Recommend the smallest movement that restores cohesion, then name the tests that protect unchanged behavior.

## Layer Leaks

Review changed code for:

- Controllers doing business workflow, Doctrine queries, external calls, or multi-write transactions.
- Services returning `Response`, reading raw `Request`, rendering Twig, or hiding persistence details.
- Repositories authorizing users, formatting API responses, or dispatching side effects.
- Entities depending on Symfony HTTP/session/container services.
- Messenger handlers, commands, subscribers, or Twig components hiding business behavior instead of delegating to services.

## Pragmatic SOLID Checks

- **SRP:** a class has one cohesive responsibility and one primary reason to change; do not use line count as a proxy.
- **OCP:** add a strategy/policy only when variants are real; reject switch-heavy mode flags and speculative plugin systems.
- **LSP:** implementations preserve contract behavior, errors, nullability, ordering, and side effects.
- **ISP:** dependencies expose only capabilities the consumer uses; split broad gateways when that reduces coupling.
- **DIP:** use narrow ports at external/volatile boundaries, not one interface for every concrete service or repository.

Compare relevant code with [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Examples are review prompts, not templates.

## Output

Lead with severity-ordered findings. Include file/line, observable impact, violated responsibility or contract, concrete movement to Controller, Service, Repository, Entity, DTO/Form, Voter, Message, or infrastructure adapter, and a regression test. If the architecture is already cohesive, say so and identify residual risk instead of inventing findings.
