# Symfony Layered Architecture Accelerator - Codex Edition

Codex uses the shared root policy, repository skills discovered under `.agents/skills`, and project integration files under `.codex`.

## Directory Map

| Piece | Location | Purpose |
| --- | --- | --- |
| Skills | `.agents/skills/<name>/SKILL.md` | Codex-discovered Symfony workflows |
| Policy | `AGENTS.md` | Shared enforceable architecture and safety rules |
| Config | `.codex/config.toml` | Trusted project configuration and feature flags |
| Hooks | `.codex/hooks.json`, `.codex/hooks/*.sh` | Context, command safety, naming, and loop checks |
| References | `.codex/DOD.md`, `.codex/GOLDEN-PRINCIPLES.md`, `.codex/STABILIZATION.md` | Completion, quality, and learning guidance |

Codex does not use duplicate `.codex/skills`, `.codex/commands`, or `.codex/agents` trees. Skills replace the deprecated project custom-prompt pattern, and ordinary collaboration/subagent support does not require Markdown wrapper files.

## Setup

1. Open the repository in Codex and trust the project.
2. Confirm repository skills are visible from `.agents/skills`.
3. Invoke a skill by name or describe work that matches its trigger description.
4. Follow root `AGENTS.md`; run `.codex/DOD.md` before claiming completion.

Use the discovered `project-brain` skill for governed shared task lifecycle, handoffs, findings/bugs/incidents/decisions, compaction, promotion proposals, and the one public task-aware retrieval command: `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID`. Governed mode is the default; `--mode lightweight` is an explicit machine-local fallback.

Use `memory-bank` only for durable retrieval/capture/audit/supersession and application of a governed automatic or independently reviewed promotion. Canonical policy, specs, code, configuration, migrations, and tests outrank all context. Codex intentionally has no `.codex/commands` or `.codex/agents` wrapper for Project Brain. Session hooks report mode, index health/staleness, active binding count, and validation status only; they never index, retrieve, load, or inject records automatically.

## Architecture

The default is `Controller -> Service -> Repository`: framework entry points stay thin, services own workflows and transactions, repositories own Doctrine queries, and input/authorization/output contracts are explicit.

The baseline supports Symfony 7.4 LTS on PHP 8.2+ and Symfony 8.1 on PHP 8.4+, while always following the consuming project's declared versions.

## Synchronization

`.agents/skills` is the configured canonical source for shared skill parity. Mirror supported Symfony workflow changes into `.claude/skills` and `.cursor/skills`, adapting paths, frontmatter, and tool-integrated mechanics such as `skill-creator` to each platform. Keep `.codex` support files aligned with the root policy and Codex's supported configuration model.
