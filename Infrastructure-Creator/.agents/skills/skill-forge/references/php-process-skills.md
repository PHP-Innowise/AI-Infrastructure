# PHP Process & Workflow Skills Reference

These are framework-agnostic process/workflow **candidates**. They are generated only when their contract's selection trigger is supported by evidence. The memory quartet is the sole exception because `memory-seed` always installs the runtime it operates.

Most are stable framework-independent workflow candidates. The only fixed inventory is the four-skill set `memory-bank`, `project-brain`, `checkpoint`, and `memory`, which operates the shared memory layer that `memory-seed` creates (the durable bank, the governed Project Brain control plane, and the context-brain runtime at `memory-bank/scripts/context.py`).

## Process & Workflow Candidate Catalog

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

Evaluate each contract independently. A generic desire for a mature workflow is not evidence. Include a candidate only when its trigger and required evidence are present in the plan, except the runtime-fixed memory quartet. Every selected candidate receives a complete JSON contract and its own authored procedure.

## Enforceable Per-Skill Contracts

Each contract defines selection, evidence, procedure, verification, output, sibling boundary, and a negative generic example. Positive examples are illustrative only and must be replaced with target facts.

### `requirements-analyst`
- **Select when / evidence:** requirements arrive through evidenced specs, tickets, ADRs, or a confirmed interview workflow; cite those locations and any acceptance-criteria convention.
- **Own / exclude:** owns ambiguity, constraints, acceptance criteria, dependencies, and open decisions; excludes solution design and implementation.
- **Procedure:** load vocabulary and source authority; decompose outcomes; trace each constraint/rule to evidence; identify contradictions and unknowns; produce testable acceptance criteria; stop for decisions that alter scope.
- **Verify / output:** every criterion is observable and every factual claim is sourced; output a requirement breakdown, assumptions, open decisions, and evidence map.
- **Sibling boundary:** `brainstorming` compares solution options; `writing-plans` sequences an approved solution.
- **Positive:** distinguish a confirmed invoice transition from a merely listed status. **Negative:** “Understand the request, make a plan, implement, test.”

### `researcher`
- **Select when / evidence:** the target has unresolved technical questions, ADR/research conventions, or dependencies requiring official versioned guidance; cite the question and internal/official sources.
- **Own / exclude:** owns evidence gathering and decision-ready findings; excludes choosing policy without authority and writing implementation.
- **Procedure:** frame answerable questions; search target sources first; use official version-matched external sources; record authority/date/version; reconcile conflicts; separate fact, inference, and recommendation.
- **Verify / output:** links resolve and conclusions are supported by at least the strongest available authority; output findings, alternatives, confidence, gaps, and recommendation criteria.
- **Sibling boundary:** `council` debates a decision after research; `requirements-analyst` decomposes requested outcomes.
- **Positive:** compare the locked package version to its official upgrade notes. **Negative:** an unsourced “best practices” list.

### `brainstorming`
- **Select when / evidence:** a requested feature has materially open product/design choices documented in the requirement or interview.
- **Own / exclude:** owns divergent alternatives and trade-off clarification; excludes pretending approval or producing implementation steps.
- **Procedure:** establish goals/non-goals; ask one consequential question at a time; propose distinct options grounded in constraints; compare trade-offs; identify reversible choices; capture the user's selected direction.
- **Verify / output:** options are meaningfully different and no decision is marked approved without user evidence; output options, comparison, recommendation, and unresolved decisions.
- **Sibling boundary:** `council` is for multi-discipline high-risk review; `writing-plans` starts only after approval.
- **Positive:** compare synchronous versus queued processing against evidenced latency needs. **Negative:** “Use the framework standard because it is best.”

### `council`
- **Select when / evidence:** an evidenced decision spans at least two risk disciplines or is explicitly high-impact/irreversible.
- **Own / exclude:** owns structured competing reviews; excludes fictional consensus, unsupported personas, and final approval.
- **Procedure:** state decision and evidence; assign only relevant lenses; have each expose risks and criteria; record disagreements; synthesize options and escalation points.
- **Verify / output:** each conclusion maps to evidence and dissent is preserved; output lens findings, conflicts, recommendation, and human decision required.
- **Sibling boundary:** `researcher` gathers facts; `security-review` performs a focused code/security review.
- **Positive:** architecture and operations challenge a migration rollout. **Negative:** five personas repeating identical generic advice.

### `writing-plans`
- **Select when / evidence:** an approved design/decision exists and the target has identifiable files, tests, and validation commands.
- **Own / exclude:** owns dependency-ordered execution steps and checkpoints; excludes redesigning or coding.
- **Procedure:** restate approved scope; map affected canonical paths; order enabling changes before dependents; pair each change with tests; place review/rollback checkpoints; identify parallel-safe work.
- **Verify / output:** another engineer can execute without guessing files, acceptance, or commands; output ordered steps with paths, tests, dependencies, and stop conditions.
- **Sibling boundary:** `requirements-analyst` defines what; `brainstorming` explores alternatives.
- **Positive:** name the migration, repository, handler, and focused test sequence. **Negative:** “Update backend, add tests, verify.”

### `using-git-worktrees`
- **Select when / evidence:** Git is present and parallel/isolated work is requested or repository conventions document worktrees/branch isolation.
- **Own / exclude:** owns safe worktree creation, environment separation, and cleanup choice; excludes branch completion and destructive cleanup.
- **Procedure:** inspect status and existing worktrees; choose evidenced branch/base; create an isolated path; copy no secrets; document per-worktree dependency/database setup; verify branch/path; request approval before removal.
- **Verify / output:** both working copies remain cleanly isolated; output paths, branches, setup state, and cleanup instructions.
- **Sibling boundary:** `finishing-branch` owns merge/PR/cleanup decisions after work is complete.
- **Positive:** isolate Composer dependencies and a test database. **Negative:** blindly copy `.env` into every worktree.

### `systematic-debugger`
- **Select when / evidence:** a reproducible defect, failing test, or observed runtime symptom exists.
- **Own / exclude:** owns hypothesis-driven root-cause methodology; excludes target observability locations and speculative fixes.
- **Procedure:** record symptom and reproduction; minimize; gather discriminating evidence; enumerate hypotheses; test one variable at a time; confirm causal mechanism; design the smallest fix and regression proof.
- **Verify / output:** the fix fails before and passes after, with alternative hypotheses ruled out; output reproduction, evidence, root cause, fix boundary, and regression test.
- **Sibling boundary:** `debugging` owns target-specific logs/APM/tools. Cross-reference it without naming those tools here.
- **Positive:** bisect a state transition with a focused failing test. **Negative:** “Try clearing cache and adding null checks.”

### `refactorer`
- **Select when / evidence:** duplication, coupling, complexity, or an approved structural objective is evidenced and behavior has a runnable safety net.
- **Own / exclude:** owns behavior-preserving structure; excludes feature changes, schema semantics, and speculative abstraction.
- **Procedure:** define preserved behavior; run focused tests; make one structural move; rerun checks; inspect public API/diff; repeat only while objective remains.
- **Verify / output:** behavior and public contracts remain stable; output refactor summary, preserved behavior, commands/results, and deferred risks.
- **Sibling boundary:** `coding` owns behavior changes; `code-review` evaluates a completed diff.
- **Positive:** extract duplicated mapping behind existing tests. **Negative:** rewrite architecture while “cleaning up.”

### `dependency-manager`
- **Select when / evidence:** Composer manifests/lock exist and the task adds, removes, audits, or upgrades dependencies.
- **Own / exclude:** owns package necessity, constraints, compatibility, advisories, lockfile impact, and removal; excludes application integration design.
- **Procedure:** inspect require versus require-dev and platform constraints; verify official compatibility; assess maintenance/security/licensing signals when evidenced; perform the narrow Composer operation; inspect transitive changes; run project checks.
- **Verify / output:** manifest-lock consistency, audit result, and target tests pass; output rationale, exact dependency changes, compatibility evidence, and commands/results.
- **Sibling boundary:** integration skills own runtime wiring; `release` owns deployment.
- **Positive:** justify a locked major upgrade from official notes. **Negative:** “Run composer update” without reviewing transitive changes.

### `review-pr`
- **Select when / evidence:** a remote PR URL/number and repository remote are available.
- **Own / exclude:** owns complete PR diff, checks, review context, and actionable findings; excludes local-only diff review and unsolicited mutation.
- **Procedure:** fetch PR metadata/comments/checks; review all commits and base diff; trace risky changes to target contracts; validate findings; classify severity; post or fix only as authorized.
- **Verify / output:** every finding cites a concrete line and consequence; output review summary, findings, checks status, and actions taken.
- **Sibling boundary:** `code-review` may review local code; `finishing-branch` prepares a branch for handoff.
- **Positive:** inspect the full branch diff and failing CI. **Negative:** review only the latest commit.

### `finishing-branch`
- **Select when / evidence:** implementation is complete on a branch and evidenced DoD/tests are available.
- **Own / exclude:** owns final checks and explicit handoff options; excludes redesign and silently merging/deleting.
- **Procedure:** inspect status/diff; run required checks; summarize changes/risks; present PR, merge, keep, or cleanup options applicable to the repository; execute only the chosen reversible action.
- **Verify / output:** status and checks are known and no destructive action is implicit; output readiness summary and chosen handoff result.
- **Sibling boundary:** `using-git-worktrees` creates isolation; `release` handles deployment/release mechanics.
- **Positive:** report failing checks before offering PR creation. **Negative:** auto-merge and delete the branch.

### `documentation-generator`
- **Select when / evidence:** canonical docs/ADR/API/changelog locations exist and a change affects their governed content.
- **Own / exclude:** owns updates to the authoritative documentation surface; excludes creating competing sources of truth.
- **Procedure:** identify authority and audience; derive facts from code/specs; update the narrow canonical location; preserve format/link conventions; test snippets/links where possible; record unresolved drift.
- **Verify / output:** documented commands and references match current files; output changed docs, authority rationale, and verification.
- **Sibling boundary:** `reflect` creates agent-behavior rules; domain skills review business invariants.
- **Positive:** update the existing ADR index after adding an ADR. **Negative:** add a second architecture guide because the first was not read.

### `skill-creator`
- **Select when / evidence:** the target has generated skill roots and the user requests a skill creation/change.
- **Own / exclude:** owns target-local skill contracts and evaluation; excludes modifying the external generator or inventing project conventions.
- **Procedure:** gather purpose/triggers/evidence/output; inspect neighboring skills; define scope and sibling boundaries; author concise progressive-disclosure content; validate frontmatter/references; test with positive and negative prompts.
- **Verify / output:** discovery triggers, non-triggers, links, and procedure are valid; output skill files and evaluation cases.
- **Sibling boundary:** `reflect` updates a rule from a correction; `documentation-generator` writes project docs.
- **Positive:** add a target-specific deployment-review skill backed by CI files. **Negative:** clone an unrelated skill and rename headings.

### `reflect`
- **Select when / evidence:** a concrete agent mistake or user correction is recorded.
- **Own / exclude:** owns durable Error -> Root Cause -> Rule -> Example -> Enforce learning; excludes vague preferences and unsupported global policy.
- **Procedure:** quote/sanitize the failure; identify controllable root cause; choose the narrow governing location; write a testable rule; add positive/negative examples; define enforcement or verification.
- **Verify / output:** the rule would have prevented the observed failure without blocking valid cases; output the changed rule and rationale.
- **Sibling boundary:** `memory-bank` stores reusable confirmed knowledge, not behavioral policy.
- **Positive:** prohibit a proven unsafe command pattern in the relevant rule. **Negative:** “Be more careful next time.”

### `memory-bank`
- **Select when / evidence:** always; required evidence is the installed `memory-bank/` runtime and section 12 seed contract.
- **Own / exclude:** owns durable reusable confirmed concepts, retrieval, audit, and supersession; excludes active task state and self-approval of promotions.
- **Procedure:** choose retrieve/capture/update/audit; revalidate canonical sources; use the shipped validator/runtime; keep one cohesive concept; preserve history and contradictions; sanitize all content.
- **Verify / output:** `memory-bank/scripts/validate.py` passes and cited sources exist; output retrieved context or changed chunk/index plus validation result.
- **Sibling boundary:** `project-brain` owns active governed work; `checkpoint`/`memory` are manual working-context controls.
- **Positive:** supersede stale architecture memory with linked evidence. **Negative:** store raw prompts, diffs, logs, or secrets.

### `project-brain`
- **Select when / evidence:** always; required evidence is `project-brain/PROTOCOL.md`, runtime config, schemas/templates, and `context.py`.
- **Own / exclude:** owns one governed task operation per invocation; excludes durable-memory application and bypassing revision/promotion controls.
- **Procedure:** read runtime mode/protocol; select exactly one supported operation; use the exact facade; enforce task ID/revision/templates; append transitions; propose but never self-approve promotion.
- **Verify / output:** runtime status and schema validation succeed; output operation, task/revision, records affected, and next legal operation.
- **Sibling boundary:** `memory-bank` owns durable knowledge; `checkpoint` cannot become a second authority in governed mode.
- **Positive:** retrieve with `--task-id` then hand off through a template. **Negative:** directly edit governed indexes.

### `checkpoint`
- **Select when / evidence:** always; required evidence is runtime mode and the shipped lightweight checkpoint command.
- **Own / exclude:** owns a manual sanitized save in lightweight mode; excludes completion, records, promotion, clear, and governed task writes.
- **Procedure:** inspect runtime mode; if governed report `working: skipped`; if lightweight, summarize branch task without raw content and invoke only the supported checkpoint command; report result.
- **Verify / output:** status confirms expected mode/state; output saved/skipped status and reason.
- **Sibling boundary:** `project-brain` is sole governed task authority; `memory` refreshes context but does not save completion.
- **Positive:** skip cleanly in governed mode. **Negative:** call destructive lifecycle commands from a save operation.

### `memory`
- **Select when / evidence:** always; required evidence is `context.py refresh`, `context.py status`, and runtime mode.
- **Own / exclude:** owns manual context refresh and health reporting; excludes task mutation, completion, promotion, or invented state.
- **Procedure:** run the shipped refresh command; run status; interpret procedural/semantic/episodic health under the authority gate; report stale/unavailable layers without repairing unsupported state.
- **Verify / output:** commands complete and health is reported; output refresh result, mode, layer health, and remediation needed.
- **Sibling boundary:** `checkpoint` saves lightweight state; `project-brain` mutates governed task state.
- **Positive:** refresh then report a stale index. **Negative:** fabricate context when retrieval is empty.
