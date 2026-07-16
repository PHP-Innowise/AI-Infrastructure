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

## Architecture

The default is `Controller -> Service -> Repository`: framework entry points stay thin, services own workflows and transactions, repositories own Doctrine queries, and input/authorization/output contracts are explicit.

The baseline supports Symfony 7.4 LTS on PHP 8.2+ and Symfony 8.1 on PHP 8.4+, while always following the consuming project's declared versions.

## Synchronization

`.claude/skills` is the canonical authored content. Mirror shared Symfony workflow changes into `.cursor/skills` and `.agents/skills`, adapting paths, frontmatter, and tool-integrated mechanics such as `skill-creator` to each platform. Keep `.codex` support files aligned with the root policy and Codex's supported configuration model.
