---
name: architecture-scanner
description: Detect a PHP target's architecture style (monolith, modular-monolith, microservices, event-driven), layering/DDD approach (layered, hexagonal/ports-and-adapters, none), module/service boundaries, and communication style from real evidence. Use as Phase 1 discovery input to profile-synthesizer. Triggers on "scan the architecture", "detect the architecture", "is this a monolith or microservices", "how is this project layered", "architecture-scanner".
phase: discovery
flow-next: profile-synthesizer
flow-alternatives: [stack-researcher]
related: [stack-scanner, integration-scanner, infra-ops-scanner, security-compliance-scanner, conventions-scanner, infra-scan]
---

# Architecture Scanner

## Overview

Read-only reconnaissance of a PHP target's architectural shape: whether it is a monolith, modular-monolith, microservices system, or event-driven design; how it is layered (classic layered, hexagonal/ports-and-adapters, DDD, or none); where module/service boundaries fall; and how components communicate (HTTP controllers, message/event buses, RPC). This scanner reads structure and dependencies only - it does not evaluate integrations, infra, security, or conventions, which belong to their own scanners.

The target project path is a **required** argument (e.g. "scan the architecture of ../my-php-app"). Never assume the current working directory is the target. Operate strictly read-only within that path; never write to it, never read its `.env`/secrets.

## Generated File Naming Convention (MANDATORY)

Write exactly one findings file into the current run's task directory: `tasks/TASK-{N}/architecture-scanner-findings.md`. Never write into the target.

## Process

1. **Map the directory tree.** Record top-level and second-level directories (`src/`, `app/`, `modules/`, `packages/`, `services/`, `domain/`, `application/`, `infrastructure/`) as the primary structural evidence, citing paths.
2. **Read the PSR-4 map in `composer.json`.** Multiple namespace roots (e.g. `App\`, `Billing\`, `Catalog\`) mapped to distinct paths signal module boundaries; a single flat root signals a plain monolith. Cite `composer.json` lines.
3. **Detect monorepo / multi-service layout.** Search for more than one `composer.json` (e.g. under `packages/*` or `services/*`); multiple runnable roots suggest modular-monolith or microservices. Cite each `composer.json` path.
4. **Classify layering/DDD.** Look for `Domain/`, `Application/`, `Infrastructure/`, `Ports/`, `Adapters/` folders (hexagonal), or `Http/`, `Service/`, `Repository/` (layered). Absence of any such split marks `inferred` "none".
5. **Detect communication style.** HTTP controllers (`Controller` classes, route files), message/event buses (`symfony/messenger`, `league/tactician`, Laravel bus/events, `php-amqplib/php-amqplib`, `enqueue/*`), and RPC/gRPC packages. Cite package + wiring where present.
6. **Infer boundaries from bounded contexts.** Correlate namespace roots, directory names, and per-module `composer.json` to name candidate boundaries; keep them `inferred` unless a manifest confirms them.
7. **Mark confidence** per finding: `confirmed` (direct evidence), `inferred` (indirect signal), or `unknown`. Never present a guess as fact.

## Output Template

```markdown
# Architecture Scanner Findings: [target_name]

**Target:** [path]  **Scanned:** [date]

## Architecture Style
- [monolith | modular-monolith | microservices | event-driven] (confirmed/inferred - evidence path:L#)

## Layering / DDD
- [layered | hexagonal/ports-and-adapters | DDD | none] (confirmed/inferred - path)

## Module / Service Boundaries
- [boundary name => namespace/path] for each (confirmed/inferred - composer.json:L# / dir)

## Monorepo Signals
- [count and paths of composer.json files, or "single root"] (confirmed - paths)

## Communication Style
- [HTTP controllers | message/event bus | RPC] (confirmed/inferred - package + wiring path:L#)

## Confidence Summary
[X confirmed, Y inferred, Z unknown]
```

## Guardrails

- MUST cite a real file path (and line where practical) for every finding.
- MUST operate read-only on the target; MUST NOT read `.env`/secrets.
- MUST base architecture claims on PHP evidence (directory tree, PSR-4 map, composer.json count, message-bus packages, layering folders).
- MUST report absent structure as `inferred none` rather than asserting a style without evidence.
- MUST NOT deep-dive integrations, infra, security, or conventions - those belong to their own scanners.

## Final Output

Return the findings file path, the detected architecture style, layering approach, candidate boundaries, and communication style, plus a one-line confidence summary. Suggest `profile-synthesizer` (to fold this into the target profile) as the next step.
