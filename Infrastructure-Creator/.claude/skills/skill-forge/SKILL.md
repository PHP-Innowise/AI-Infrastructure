---
name: skill-forge
description: Generate the target PHP project's full custom SKILL.md set (an architecture skill, universal PHP skills, and one skill per confirmed integration) from an approved Project Profile, for the selected edition(s). Use once profile-synthesizer has produced a profile. Triggers on "generate skills", "forge the target's skills", "write SKILL.md for the target".
phase: generation
flow-next: agent-forge
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Skill Forge

## Overview

`skill-forge` writes the target project's operational skills: an architecture skill grounded in the detected pattern, the universal PHP skills every project needs, and exactly one skill per `confirmed` integration. Every skill is authored from the profile's evidence - never templated - and reflects the target's real PHP framework, version, and tooling. It writes only into the selected edition(s)' `skills/` tree(s) inside the target.

Consult the bundled references for PHP-specific grounding:
- `references/php-frameworks.md` - detection signals and baseline skill scaffolding per PHP framework.
- `references/php-architecture-patterns.md` - architecture detection and its generated-skill implications.
- `references/php-integration-catalog.md` - integration categories and what good coverage looks like per category.

## Generated File Naming Convention (MANDATORY)

Into the target, for each selected edition, write `<edition-skills-dir>/<skill-name>/SKILL.md` where the edition skills dir is `.claude/skills`, `.cursor/skills`, and/or `.agents/skills`. Also append a generation log to the run's `tasks/TASK-{N}/skill-forge-log.md` listing every skill produced.

## Process

1. **Read the profile** (sections 2-4, and section 10's "Skills to generate"). Re-confirm the selected editions from section 1.
2. **Generate the architecture skill** from section 3, using `references/php-architecture-patterns.md` to shape guidance appropriate to the detected pattern (monolith / modular-monolith / microservices / event-driven) and layering.
3. **Generate the universal PHP skills** adapted to the detected framework/version and tooling (from `references/php-frameworks.md`):
   - `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`.
   - Each references the target's REAL tools (e.g. the actual test runner and static-analysis config found), not assumed defaults.
4. **Generate one skill per confirmed integration** in section 4, using `references/php-integration-catalog.md` for what good coverage looks like per category (payment, queue, search, cache, storage, email, auth, observability, etc.). Skip `inferred`/`unknown` integrations - or generate them only if the interview confirmed them.
5. **Write valid frontmatter** for each skill: `name`, `description` (one line with triggers), `phase`, `flow-next`, `flow-alternatives`, `related`. Cross-references in `related`/`flow-next` MUST point only at skills this run actually generates.
6. **Carry evidence forward.** Each generated skill notes the source facts it is grounded in (file paths), so the output is auditable back to the scan.
7. **Log** every generated skill to `skill-forge-log.md` (used by `agent-forge`, `command-forge`, and `skill-flow-composer`).

## Output Template

```markdown
# Skill Forge Complete: [target_name]

**Editions:** [selected]
**Skills generated:** [count]
- architecture: [name]
- universal: [list]
- integrations: [list]

## Log
tasks/TASK-{N}/skill-forge-log.md

## Next
agent-forge (wrap these skills), then command-forge; policy-forge/hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST author from profile evidence; MUST NOT template or invent a skill for an absent integration.
- MUST reflect the target's real framework/version and real tooling, not assumed defaults.
- MUST write only the selected edition(s).
- MUST ensure every cross-reference resolves to a skill generated in this run.
- MUST NOT generate deep skills for non-PHP neighbors - those are integration contracts only.

## Final Output

Return the editions written, the full generated skill list by category, the log path, and the next step (`agent-forge`).
