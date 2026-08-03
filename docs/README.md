# User Documentation

PHP AI Accelerators adds agent policy, workflows, safety hooks, and local
context tooling to an existing PHP project. Start with the goal that matches
your situation.

## Choose a Starting Point

- **Install a ready-made edition safely:** [Adoption Guide](ADOPTION.md)
- **Configure Claude Code, Cursor, or Codex:** [Tool Integrations](TOOL-INTEGRATIONS.md)
- **Choose governed or lightweight context:** [Context Modes](CONTEXT-MODES.md)
- **Operate Project Brain and memory:** [Context and Memory Operations](OPERATIONS.md)
- **Understand security boundaries:** [Security and Trust Boundaries](SECURITY.md)
- **Diagnose installation or runtime problems:** [Troubleshooting](TROUBLESHOOTING.md)
- **Add or change accelerator behavior:** [Extending the Accelerator](EXTENDING.md)
- **See a complete task lifecycle:** [User Task Workflow Example](examples/USER-TASK-WORKFLOW-EXAMPLE.md)
- **Compare the repository in English or Russian:** [English overview](../README_EN.md) · [Russian overview](../README_RU.md)

## Choose an Edition

- [Laravel](../Laravel/README.md) — Laravel 12/13, Eloquent, Artisan, queues,
  events, notifications, Filament, and package development.
- [Symfony](../Symfony/README.md) — Symfony 7.4 LTS or 8.1, Doctrine,
  Messenger, API Platform, Forms, voters, and Symfony UX.
- [PHP Core](../PHP%20Core/README.md) — framework-neutral PHP 8.2+,
  Composer/PSR projects, PDO, or frameworks without a dedicated edition.
- [Infrastructure-Creator](../Infrastructure-Creator/README.md) — generate a
  project-specific accelerator from observed PHP project evidence instead of
  adopting a generic edition.

The ready-made editions are workflow layers, not generated applications. Open
the selected edition as the workspace root or install it at the real
project root; tools do not discover an edition nested elsewhere.

## Navigate by Tool

- **Claude Code:** `.claude/` provides commands, agent wrappers, skills,
  settings, and hooks.
- **Cursor:** `.cursor/` is a self-contained native edition, except that it
  cannot receive a Task Capsule automatically - see the
  [tool capability matrix](TOOL-INTEGRATIONS.md). It provides commands,
  agents, skills, rules, and hooks.
- **Codex:** `.agents/skills/` provides discovered skills; `.codex/` provides
  trusted project configuration, hooks, and reference documents.

For exact ownership, activation checks, and version caveats, use the
[Tool Integrations guide](TOOL-INTEGRATIONS.md).

## Source-of-Truth Hierarchy

When sources disagree, use this order:

1. Enforcement: active hooks, CI, linters, and static analysis.
2. Policy: the installed root `AGENTS.md`.
3. Current project truth: specifications, code, configuration, migrations,
   and tests.
4. Verified durable knowledge in `memory-bank/`.
5. Operational workflows in the active tool's skills.
6. Examples.
7. Human documentation.

Project Brain coordinates current work but does not override policy or current
project truth. Retrieved context is a discovery aid and must be checked
against its cited source. The runtime configuration in every ready-made
edition declares `.agents` as the canonical skill edition; tool-specific
commands, agents, rules, settings, and hooks remain native adapters.

## Next Steps

1. Select an edition from the project’s actual framework and declared
   versions.
2. Follow the [Adoption Guide](ADOPTION.md), including backup, collision
   review, local ignores, and validation.
3. Activate only the tool integrations the team uses.
4. Use [Context Modes](CONTEXT-MODES.md) before changing the default governed
   context behavior.
