---
name: skill-forge
description: Generate the target PHP project's full custom SKILL.md set - architecture, design & interaction, conditional frontend, framework-agnostic process/workflow, universal PHP, evidence-driven framework-specialty, and one skill per confirmed integration - from an approved Project Profile, for the selected edition(s). Use once profile-synthesizer has produced a profile. Triggers on "generate skills", "forge the target's skills", "write SKILL.md for the target".
phase: generation
flow-next: agent-forge
flow-alternatives: [policy-forge, hook-forge, memory-seed]
related: [infra-generate, policy-forge, agent-forge, command-forge, hook-forge, memory-seed, skill-flow-composer, bootstrap-verifier]
---

# Skill Forge

## Overview

`skill-forge` writes the target project's full operational skill set across six groups: the architecture skill; the always-generated design & interaction skills; the frontend group when a UI surface exists; the 14 framework-agnostic process/workflow skills every project needs; the 7 universal PHP skills; the evidence-driven framework-specialty skills; and exactly one skill per `confirmed` integration. Every non-fixed skill is authored from the profile's evidence - never templated - and reflects the target's real PHP framework, version, and tooling. It writes only into the selected edition(s)' `skills/` tree(s) inside the target.

This breadth exists so a generated target ends up with an accelerator that matches the depth of the hand-built Laravel/Symfony/PHP-Core accelerators in this monorepo - not just an architecture skill plus a handful of generic ones - while staying 100% evidence-driven rather than templated from any specific framework folder.

Consult the bundled references for PHP-specific grounding:
- `references/php-frameworks.md` - detection signals, baseline skill scaffolding per PHP framework, and the always-generated Design & Interaction / conditional Frontend skill groups.
- `references/php-architecture-patterns.md` - architecture detection and its generated-skill implications.
- `references/php-integration-catalog.md` - integration categories and what good coverage looks like per category.
- `references/php-process-skills.md` - the 14 always-generated, framework-agnostic process/workflow skills.
- `references/php-specialty-skills.md` - the evidence-gated framework-specialty catalog, keyed to profile section 3.1's signals.

## Generated File Naming Convention (MANDATORY)

Into the target, for each selected edition, write `<edition-skills-dir>/<skill-name>/SKILL.md` where the edition skills dir is `.claude/skills`, `.cursor/skills`, and/or `.agents/skills`. Also append a generation log to the run's `tasks/TASK-{N}/skill-forge-log.md` listing every skill produced, grouped exactly as below.

## Process

1. **Read the profile** (sections 2, 3, 3.1, 3.2, 4, and section 10.1's "Skills To Generate" - its draft descriptions are a useful starting point, not a substitute for authoring from evidence). Re-confirm the selected editions from section 1.
2. **Generate the architecture skill** from section 3, using `references/php-architecture-patterns.md` to shape guidance appropriate to the detected pattern (monolith / modular-monolith / microservices / event-driven) and layering.
3. **Generate the three Design & Interaction skills** - `architecture-implementer`, `api-designer`, `database-designer` - always, per `references/php-frameworks.md`'s "Design & Interaction Skills" section, grounded in the target's real scaffolding tooling, API shape, and persistence layer.
   - `database-designer` owns schema/entity/index/migration DESIGN only; it MUST cross-reference (not duplicate) `orm-patterns` for ORM usage patterns when that specialty skill is also generated (step 7).
   - `api-designer` MUST check whether `api-platform-design` will also be generated (a confirmed declarative API framework in section 3.1); if so, narrow `api-designer` to any remaining hand-rolled endpoints and explicitly defer resource-level design to `api-platform-design` rather than duplicating it.
4. **Generate the Frontend group conditionally** on section 3.2's verdict: if it applies, generate `frontend-design`, `coder-frontend`, `wcag-accessibility`, `web-design-guidelines`, `browser-verify` per `references/php-frameworks.md`'s "Frontend Skills" section, grounded in the target's real templating/asset stack; if the verdict says no UI surface, generate none of these five and say so in the log.
5. **Generate the 14 Process & Workflow skills** always, per `references/php-process-skills.md`: `requirements-analyst`, `researcher`, `brainstorming`, `council`, `writing-plans`, `using-git-worktrees`, `systematic-debugger`, `refactorer`, `dependency-manager`, `review-pr`, `finishing-branch`, `documentation-generator`, `skill-creator`, `reflect`. Author each from the target's real conventions where evidence exists (section 7's docs locations, the actual git remote, etc.); fall back to the sound generic mechanic in the reference where no project-specific convention was found.
6. **Generate the 7 universal PHP skills** adapted to the detected framework/version and tooling (from `references/php-frameworks.md`):
   - `coding`, `testing`, `code-review`, `security-review`, `performance`, `release`, `debugging`.
   - Each references the target's REAL tools (e.g. the actual test runner and static-analysis config found), not assumed defaults.
   - `debugging` and the process skill `systematic-debugger` are a deliberate split, not a duplicate: `debugging` owns only "where to look in this target" (real error tracker/logs/in-app tools per `references/php-frameworks.md`'s "`debugging`'s Scope" note) and MUST cross-reference `systematic-debugger` for the investigative methodology rather than re-stating it; `systematic-debugger` MUST stay tool-agnostic and cross-reference `debugging` for where to apply it in this target. Set each other in `related` frontmatter.
   - `performance` and the specialty skill `caching-strategy` (when generated) are a deliberate split, not a duplicate: `performance` names caching only as one hot-path lever among several and MUST cross-reference `caching-strategy` for cache-correctness depth rather than re-deriving it; only cover cache guidance directly in `performance` when `caching-strategy` is NOT generated (no signal in section 3.1).
7. **Generate the Framework-Specialty skills**, one per `confirmed`/`inferred` **present** signal in profile section 3.1 (never for a signal that is `confirmed`/`inferred` **absent**, e.g. "inferred none" or "code-driven bindings only" when the row requires the opposite - see `references/php-specialty-skills.md`'s Generation Rule), per its mapping table. Skip every `none`/`unknown` signal - never generate one speculatively, and never name a specialty skill after a framework the target does not use (name it after the generic concept, e.g. `orm-patterns` or `eloquent-patterns` if Eloquent specifically is confirmed - not a Symfony/Doctrine name for a Laravel target). Cross-check each specialty skill's scope note against its Design & Interaction / Universal counterpart (`orm-patterns` vs. `database-designer`; `caching-strategy` vs. `performance`; `api-platform-design` vs. `api-designer`) so the two never duplicate the same content.
8. **Generate one skill per confirmed integration** in section 4, using `references/php-integration-catalog.md` for what good coverage looks like per category (payment, queue, search, cache, storage, email, auth, observability, etc.). Skip `inferred`/`unknown` integrations - or generate them only if the interview confirmed them.
9. **Write valid frontmatter** for each skill: `name`, `description` (one line with triggers), `phase`, `flow-next`, `flow-alternatives`, `related`. Cross-references in `related`/`flow-next` MUST point only at skills this run actually generates.
10. **Carry evidence forward.** Each generated skill (other than the 14 fixed process/workflow skills, which cite the project-specific convention they were grounded in only where one exists) notes the source facts it is grounded in (file paths), so the output is auditable back to the scan.
11. **Log** every generated skill, grouped by category (architecture / design & interaction / frontend / process & workflow / universal / specialty / integrations), to `skill-forge-log.md` (used by `agent-forge`, `command-forge`, and `skill-flow-composer`).

## Output Template

```markdown
# Skill Forge Complete: [target_name]

**Editions:** [selected]
**Skills generated:** [total count] (1 architecture + 3 design + [0 or 5] frontend + 14 process + 7 universal + [N] specialty + [M] integrations)
- architecture: [name]
- design & interaction: architecture-implementer, api-designer, database-designer
- frontend: [list, or "skipped - no UI surface detected"]
- process & workflow: [list of 14]
- universal: [list of 7]
- specialty: [list, or "none - no confirmed/inferred signals in section 3.1"]
- integrations: [list]

## Log
tasks/TASK-{N}/skill-forge-log.md

## Next
agent-forge (wrap these skills), then command-forge; policy-forge/hook-forge/memory-seed if not already run.
```

## Guardrails

- MUST author every non-fixed skill from profile evidence; MUST NOT template or invent a skill for an absent integration or an un-confirmed specialty signal.
- MUST generate all 14 process & workflow skills for every target, and all 3 design & interaction skills for every target - these are not conditional.
- MUST generate the 5 frontend skills only when section 3.2's verdict applies; MUST NOT generate them for a target with no UI surface.
- MUST reflect the target's real framework/version and real tooling, not assumed defaults.
- MUST write only the selected edition(s).
- MUST ensure every cross-reference resolves to a skill generated in this run.
- MUST NOT generate deep skills for non-PHP neighbors - those are integration contracts only.
- MUST NOT let a scope-split pair duplicate content: `debugging`/`systematic-debugger`, `database-designer`/`orm-patterns`, `performance`/`caching-strategy`, and `api-designer`/`api-platform-design` each have one explicit owner per concern and MUST cross-reference, not restate, their counterpart's half.

## Final Output

Return the editions written, the full generated skill list by all seven groups (architecture / design & interaction / frontend / process & workflow / universal / specialty / integrations), the log path, and the next step (`agent-forge`).
