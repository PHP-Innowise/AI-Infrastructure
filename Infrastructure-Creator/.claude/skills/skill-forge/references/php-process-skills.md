# PHP Process & Workflow Skills Reference

These are the **always-generated, framework-agnostic** process/workflow skills. Unlike the universal PHP skills (`references/php-frameworks.md`) or the framework-specialty skills (`references/php-specialty-skills.md`), these do not change shape based on which PHP framework or integrations the target uses - the same 18 apply to every PHP project. `skill-forge` still authors each one grounded in the target's real conventions where one exists rather than pasting generic boilerplate - only the underlying mechanic is fixed, not the wording.

Fourteen are stable framework-independent workflow mechanics. The remaining four - `memory-bank`, `project-brain`, `checkpoint`, and `memory` - operate the shared memory layer that `memory-seed` creates (the durable bank, the governed Project Brain control plane, and the context-brain runtime at `memory-bank/scripts/context.py`).

## The 18 Process & Workflow Skills

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
| `memory-bank` | Operates the shared `memory-bank/` after `memory-seed` creates it: retrieve and revalidate active context, capture cohesive confirmed concepts, supersede stale memory without erasing history, and audit structure/source freshness. Its full contract is in `references/php-domain-behavior.md`. | utility |
| `project-brain` | Operates the governed `project-brain/` control plane through the runtime facade `memory-bank/scripts/context.py`: shared task lifecycle and handoffs, governed retrieval (`retrieve QUERY --task-id ID`), findings/bugs/incidents/decisions/events records, compaction, and promotion proposals. One operation per invocation; canonical sources always outrank retrieved context. | utility |
| `checkpoint` | Manually saves current working state without completing anything. Governed-aware: when `project-brain/config/runtime.json` says `governed`, it reports `working: skipped` and defers to `project-brain` (one task authority, never two); only in explicit lightweight mode does it checkpoint the branch task via `context.py --mode lightweight` with a sanitized summary - never raw diffs, file bodies, or secrets. | utility |
| `memory` | Manually refreshes repository-local context and reports layer health: runs `context.py refresh` (procedural/semantic/episodic - the same command the read hook runs, so skill and hook cannot drift apart) and `context.py status`, honors the governed/lightweight authority gate, and never completes/promotes/invents task state. | utility |

Note: `release` is generated as part of the universal PHP set (`references/php-frameworks.md`) since its content already names the target's real CI/CD pipeline; it is not duplicated here.

**`systematic-debugger` vs. `debugging` - deliberately split, not duplicated.** Both skills touch "debugging" but own strictly different halves, and each must explicitly link to the other in its `related` frontmatter and its body so a reader lands on the right one:
- `systematic-debugger` (this list, process) = **how** to debug: the investigative discipline, valid for any PHP project regardless of what tooling it has.
- `debugging` (`references/php-frameworks.md`, universal) = **where** to debug in THIS target: its real error tracker/APM (e.g. Sentry), its real log locations/format, and any real in-app debugging tool it ships (Xdebug config, Laravel's `ray()`/`dd()`, Symfony's `VarDumper`, etc.).
`skill-forge` MUST NOT let either skill re-explain the other's half - `debugging` assumes the reader already knows the root-cause discipline and links to `systematic-debugger` for it; `systematic-debugger` names zero target-specific tools and links to `debugging` for where to apply that discipline in this target.

## The Memory Quartet (`memory-bank`, `project-brain`, `checkpoint`, `memory`)

These four operate one shared layer and must be authored as a coherent set, never in isolation:

- **Division of authority.** `project-brain/` owns active work (tasks, handoffs, findings, bugs, incidents, decisions, events); `memory-bank/` owns durable reusable knowledge; the SQLite index under `memory-bank/local/` is a disposable cache. Canonical policy, specs, code, migrations, and tests outrank all of it. Every one of the four skills must state this hierarchy.
- **`project-brain`** is the governed control plane skill: one operation per invocation (start/bind, retrieve, update/handoff, record, complete, compact, propose promotion, lightweight); the public retrieval interface is exactly `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID`; append-only transitions, optimistic revisions, and templates under `project-brain/templates/` govern every record; an agent proposes promotions but never approves or applies its own.
- **`checkpoint`** and **`memory`** are manual, argument-free companions to the automatic hooks `hook-forge` wires (`working-memory-write.sh` / `working-memory-read.sh`). Author them to run the same CLI commands the hooks run (`turn`-adjacent lightweight checkpointing, `refresh`, `status`) so the manual and automatic paths cannot drift. Both must respect the authority gate: governed mode means Project Brain is the only task authority.
- **Safety, shared by all four:** never store raw prompts/responses/diffs/logs/secrets/customer data; never run destructive lifecycle commands (`complete`, `record`, `clear`) from a save/refresh skill; only ignored local state may be touched outside supported CLI mutations.
- **Frontmatter linking:** the four reference each other in `related` (and `memory-bank`/`project-brain` also relate to `reflect` and `documentation-generator`); do not point them at skills the target does not receive.

`memory-seed` installs the runtime these skills drive (`context.py`, `brain_runtime.py`, `context_retrieval.py`, `validate.py`, and the `project-brain/` skeleton with `PROTOCOL.md`). Author the quartet against those real shipped files - the CLI subcommands and paths are fixed contracts, so name them exactly and invent no flags.

## Generation Rule

`skill-forge` generates all 18 for every target, regardless of framework or detected integrations. Author each from the profile's real conventions where evidence exists (git remote for `review-pr`, doc locations from profile section 7 for `documentation-generator`, the target's actual branch/worktree conventions for `using-git-worktrees`, and section 12's memory contract for `memory-bank` and the rest of the quartet), and fall back to the sound generic mechanic described above where no project-specific convention was found - never invent a convention that isn't evidenced.
