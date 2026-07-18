# PHP AI Accelerators

A collection of framework-specific AI coding-agent accelerators for PHP teams. Each top-level folder is a **complete, self-contained accelerator** for one PHP stack — structured commands, single-purpose agents, reusable skills, quality gates, and documentation conventions — usable from **Claude Code**, **Cursor**, and **OpenAI Codex**.

```
accelerator-php/
├── Laravel/     # Laravel-first accelerator
├── Symfony/     # Symfony-first accelerator
└── PHP Core/    # Framework-agnostic native PHP accelerator
```

These three used to live on separate branches (`feature/laravel-accelerator`, `feature/symfony-accelerator`, `main`). They are now grouped as sibling folders in one place so the whole department can browse, compare, and pick the right one without switching branches.

## Which One Do I Use?

| Folder | Framework | PHP baseline | Skills | Best for |
| --- | --- | --- | --- | --- |
| [`Laravel/`](./Laravel/README.md) | Laravel 12 / 13 | 8.2+ (8.3+ for Laravel 13) | 39 | Projects built on Laravel: Eloquent, Artisan, Sanctum, Filament, queues, and the wider Laravel package ecosystem. |
| [`Symfony/`](./Symfony/README.md) | Symfony 7.4 LTS / 8.1 | 8.2+ (8.4+ for 8.1) | 43 | Projects built on Symfony: Controller -> Service -> Repository, Doctrine, Messenger, API Platform, Voters. |
| [`PHP Core/`](./PHP%20Core/README.md) | None (framework-agnostic) | 8.2+ | 30 | Plain PSR-based PHP, a framework not covered here, or as the universal reference the other two specialize from. |

If your project already runs Laravel or Symfony, use that folder directly. Use `PHP Core/` for anything else — vanilla PHP, a microframework, or a framework this repo doesn't have a dedicated edition for yet.

## Using an Accelerator In Your Project

Claude Code, Cursor, and Codex all auto-discover their config (`.claude/`, `.cursor/`, `.agents/` + `.codex/`) starting from the folder your editor/CLI treats as the project root. Since each accelerator's config lives one level down (e.g. `Laravel/.claude/`), pick one of these two approaches:

1. **Open the accelerator folder itself as your workspace root** — e.g. open `Laravel/` directly in Cursor/Claude Code/Codex, with your actual Laravel application checked out alongside or inside it.
2. **Copy the folder's contents into your project's root** — copy everything inside `Laravel/` (`.claude/`, `.cursor/`, `.agents/`, `.codex/`, `AGENTS.md`, etc.) into the root of your real application repository.

Opening the monorepo root (`accelerator-php/`) itself will not auto-load any edition, since none of the tools look for config nested under `Laravel/`, `Symfony/`, or `PHP Core/`.

## Shared Architecture: Command -> Agent -> Skill

All three accelerators follow the same workflow model, just specialized for a different stack:

```
User runs: /some-command [prompt]
              |
              v
      Command selects an agent
              |
              v
      Agent executes one skill in isolated context
              |
              v
      Output: result + context summary + next steps
```

- **Commands** route user intent to the right agent (Claude Code / Cursor only — Codex invokes skills directly by name).
- **Agents** are thin wrappers that run exactly one skill, then stop, keeping the main conversation clean and the user in control.
- **Skills** contain the actual workflow: examples, checklists, decision guidance, and output templates.
- **Hooks** enforce naming, safety, and verification conventions per tool.
- Every folder keeps its own `tasks/` (temporary, skill-prefixed task docs) and `specs/` (permanent living specifications, indexed by `specs/MANIFEST.md`).

## Multi-Tool Editions (per folder)

Each accelerator folder mirrors the same skills across three tools so they coexist without conflict:

| Tool | Reads | Notes |
| --- | --- | --- |
| **Claude Code** | `.claude/` | Original edition: agents, commands, hooks, skills, `settings.json`. |
| **Cursor** | `.cursor/` | Self-contained mirror: skills, commands, agents, `rules/*.mdc`, `hooks.json`. Keep Cursor's "read `.claude`" setting **off** to avoid double-loading. |
| **Codex** | `.agents/skills/` + `.codex/` | Skills live in `.agents/skills` (the path Codex discovers); `.codex/` holds `config.toml`, `hooks.json`, and references. No command layer — invoke a skill by name. |

Each folder's own `AGENTS.md` is the enforceable policy for that stack; its `README.md` documents the full directory layout, prerequisites, quick-start command table, and verification steps in detail.

## What's Different Between Editions

- **Laravel** adds Laravel-only skills with no Symfony/native-PHP equivalent: `filament`, `eloquent`, `queues-jobs`, `events-notifications`, `auth-scaffolding`, `caching`, `console-scheduler`, `file-storage`, `package-developer`.
- **Symfony** adds Symfony-only skills: `api-platform-designer`, `doctrine-migration-designer`, `event-subscriber-designer`, `form-validator-designer`, `security-voter-designer`, `messenger-designer`, `console-command-coder`, `fixture-factory-generator`, `architecture-boundary-reviewer`, `repository-reviewer`, `container-reviewer`, `twig-ux-reviewer`. It's also the only edition with a `memory-bank/` — an indexed, cross-session, source-verified project-memory store shared by all three tools.
- **PHP Core** is deliberately the smallest and most conservative: no framework assumed, so no ORM/router/DI-container skills — just the framework-agnostic essentials (architecture, API design, database design, coding, testing, review, security, performance, dependency management, debugging, release).

Each folder's `CHANGELOG.md` tracks its own version history independently — the three no longer need to be merged or kept in lockstep now that they aren't sharing a branch.

## Contributing

- Change a skill only inside the folder(s) it applies to; do not assume a fix in `Laravel/` also belongs in `Symfony/` or `PHP Core/` — verify against that stack's own conventions first.
- When a skill or fix is genuinely universal (applies the same way regardless of framework), consider whether it belongs in `PHP Core/` and should be adapted (not copy-pasted) into the framework-specific editions.
- Within a single folder, mirror any skill/agent/command edit across that folder's `.claude/`, `.cursor/`, and `.agents/`/`.codex/` editions, and record the change in that folder's `CHANGELOG.md`.
