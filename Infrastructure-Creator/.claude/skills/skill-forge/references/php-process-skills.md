# PHP Process & Workflow Skills Reference

These are the **always-generated, framework-agnostic** process/workflow skills. Unlike the universal PHP skills (`references/php-frameworks.md`) or the framework-specialty skills (`references/php-specialty-skills.md`), these do not change shape based on which PHP framework or integrations the target uses - the same 14 apply to every PHP project, from a plain-PHP script to a Symfony microservice. `skill-forge` still authors each one grounded in the target's real conventions where one exists (e.g. `writing-plans`' file-naming convention, `using-git-worktrees`' real `.env`/DB isolation steps, `review-pr`'s real `gh` remote) rather than pasting generic boilerplate - only the underlying *mechanic* is fixed, not the wording.

Derived from auditing the equivalent skills across the Laravel, Symfony, and PHP Core accelerators in this monorepo: these 14 are the ones whose content was found to be identical or near-identical across all three, i.e. genuinely framework-independent.

## The 14 Process & Workflow Skills

| Skill | What it does | Phase |
| --- | --- | --- |
| `requirements-analyst` | Parses a requirement (a written spec, a ticket, a user ask) into a decomposed, validated task breakdown before any design or code work starts. | understanding |
| `researcher` | Turns an open question into a sourced, decision-ready findings doc; separates internal-codebase research (grep/read the target) from external research (official docs, changelogs). | understanding |
| `brainstorming` | Mandatory pre-creative-work dialogue: turns a vague idea into a concrete spec through iterative clarifying questions before any implementation begins. | understanding |
| `council` | Convenes a simulated multi-persona expert panel (architecture, security, performance, testing, ops) to debate a high-stakes or ambiguous decision before committing to it. | planning |
| `writing-plans` | Converts an approved design/decision into a step-by-step, dependency-ordered implementation plan that another engineer (or agent) could execute without re-deriving the architecture. | planning |
| `using-git-worktrees` | Sets up an isolated git worktree (own checkout, own `.env`/DB where relevant) so experimental or parallel implementation work never collides with the main working copy. | planning |
| `systematic-debugger` | The debugging **methodology**, tool-agnostic: reproduce, isolate, and confirm the actual root cause before applying a fix - forbids guessing or symptom-only patches. Deliberately contains no target-specific tool names; it stays valid whether the target is instrumented with Sentry, plain error logs, or nothing at all. Complements, and is cross-referenced by, the universal `debugging` skill (see the note below and `references/php-frameworks.md`), which supplies the *"where to look"* half. | execution |
| `refactorer` | Behavior-preserving structural cleanup performed under a test safety net: extract methods/services, remove duplication, and (only where evidence supports it) apply automated refactoring tooling. | execution |
| `dependency-manager` | Composer dependency hygiene: vulnerability audits (`composer audit`), outdated-package review, tightening version constraints, and vetting new packages before they're added. | execution |
| `review-pr` | Reviews a **remote** pull request via the `gh` CLI (not a local diff) and either fixes flagged issues locally or posts review comments back to the PR. | execution |
| `finishing-branch` | The end-of-implementation decision point: once tests/DoD pass, presents structured options (open a PR, merge, clean up the worktree) rather than silently picking one. | execution |
| `documentation-generator` | Generates and maintains README sections, ADRs, API docs, and changelog entries so documentation tracks the code instead of drifting from it. | finalization |
| `skill-creator` | Meta-skill for creating, editing, and evaluating the target's OWN skills after generation - lets the project's team extend its generated accelerator safely once Infrastructure-Creator has handed it off. | utility |
| `reflect` | Converts an agent mistake or a user correction into a permanent rule via an Error -> Root Cause -> Rule -> Example -> Enforce cycle, so the same mistake isn't repeated in a later session. | utility |

Note: `memory-bank` (the skill used to retrieve/capture/audit project memory day to day, as opposed to `memory-seed` which bootstraps the folder once at generation time) is generated alongside these 14 whenever `memory-seed` has run - see `memory-seed/SKILL.md`. `release` is generated as part of the universal PHP set (`references/php-frameworks.md`) since its content already names the target's real CI/CD pipeline; it is not duplicated here.

**`systematic-debugger` vs. `debugging` - deliberately split, not duplicated.** Both skills touch "debugging" but own strictly different halves, and each must explicitly link to the other in its `related` frontmatter and its body so a reader lands on the right one:
- `systematic-debugger` (this list, process) = **how** to debug: the investigative discipline, valid for any PHP project regardless of what tooling it has.
- `debugging` (`references/php-frameworks.md`, universal) = **where** to debug in THIS target: its real error tracker/APM (e.g. Sentry), its real log locations/format, and any real in-app debugging tool it ships (Xdebug config, Laravel's `ray()`/`dd()`, Symfony's `VarDumper`, etc.).
`skill-forge` MUST NOT let either skill re-explain the other's half - `debugging` assumes the reader already knows the root-cause discipline and links to `systematic-debugger` for it; `systematic-debugger` names zero target-specific tools and links to `debugging` for where to apply that discipline in this target.

## Generation Rule

`skill-forge` generates all 14 of these for every target, regardless of framework or detected integrations. Author each from the profile's real conventions where evidence exists (git remote for `review-pr`, doc locations from profile section 7 for `documentation-generator`, the target's actual branch/worktree conventions for `using-git-worktrees`), and fall back to the sound generic mechanic described above where no project-specific convention was found - never invent a convention that isn't evidenced.
