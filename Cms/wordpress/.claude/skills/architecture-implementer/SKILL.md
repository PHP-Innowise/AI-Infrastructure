---
name: architecture-implementer
description: Scaffold an approved WordPress architecture into plugin/theme bootstrap, hook registration, services, adapters, block/REST/CLI entry points, tests, and Composer wiring without implementing feature behavior.
phase: execution
flow-next: coder
flow-alternatives: [plugin-development, test-generator, code-reviewer]
related: [architect, hooks-events, rest-api, block-development]
---

# WordPress Architecture Implementer

## Boundary

Turn an approved architecture/spec into a compiling structural skeleton. The
architect decides; this skill scaffolds; implementation skills fill behavior.
If boundaries, data ownership or lifecycle remain undecided, stop and return to
`architect`.

## Procedure

1. Read the approved decision, current bootstrap, Composer autoload, WordPress
   and PHP minimum versions, tests, naming conventions and deployment package.
2. Create only the agreed structure: bootstrap/composition root, hook
   registrars/subscribers, thin adapters, cohesive services, real gateway
   interfaces, REST controllers, blocks, CLI commands or repositories.
3. Wire registration onto correct WordPress lifecycle hooks without executing
   request behavior at include time. Preserve existing plugin/theme entry files
   and globally visible identifiers.
4. Add interfaces only at external/substitution boundaries such as HTTP
   clients, clocks, gateways, storage or multiple implementations. Do not wrap
   every WordPress function mechanically.
5. Use typed placeholders or narrow TODOs without returning plausible fake
   success. Do not add business rules, migrations or destructive lifecycle
   behavior in the scaffold.
6. Add a smoke/registration test or compile-level seam proving autoload and
   hook wiring. Run syntax, autoload, standards/static analysis and focused
   tests supported by the project.

## Handoff

Return a table of files, responsibility, lifecycle hook, dependency direction,
and exact behavior still TODO; verification evidence; Context Summary; and the
narrowest implementation skill (`coder`, `plugin-development`, `rest-api`,
`block-development`, or another specialty).
