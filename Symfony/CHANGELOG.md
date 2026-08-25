# Changelog

Shared-core history - the Python memory/context core
(`memory-bank/scripts`, `memory-bank/tests`, `project-brain/`), the
tool hooks, and the mirror machinery common to the Laravel, Symfony
and PHP Core editions - is recorded once in the root
[`CHANGELOG.md`](../CHANGELOG.md). This file records only
Symfony-specific changes. The edition's release version is the
`VERSION` file beside this changelog; the SessionStart hook prints
it at the top of every session.

## Unreleased

### Added

- **Workflow skills now produce and retire durable knowledge, and the
  procedural rules gained a lifecycle** (roadmap H2-02, H2-03, H2-04, H2-05,
  H2-07; the engine changes behind them are in the root changelog).
  - `memory-bank` skill: retiring a chunk is `context.py bank-retire`, not a
    hand edit of frontmatter on both sides of a replacement link, and a chunk
    written by hand is followed by `bank-reverify` so it can later notice that
    what it cites has moved on. The two questions `valid_to` and
    `superseded_by` answer are spelled out, because the field was named in no
    skill and an agent following policy honestly set a status and never wrote
    a date.
  - `/flow-review` materializes each confirmed finding as a `finding` record —
    from the orchestrator after synthesis, not from the review agents, which
    stay read-only so the stage keeps exactly one write-capable agent. Until
    now a review wrote its findings into the task's `--progress`, and the
    `task` record type can never be promoted, so nothing a review produced
    could ever become durable memory.
  - `systematic-debugger` records its confirmed root cause directly (it is
    already declared write-capable); `code-reviewer` and `security-reviewer`
    gained the opposite instruction, so the division is stated where each
    skill is read rather than inferred.
  - `DOD.md`, Standard tier: every confirmed review or debugging finding
    exists as a `finding` record — resolved, or explicitly deferred.
  - `reflect` skill and `STABILIZATION.md`: the rule template gained
    `Retired:` and `Superseded-by:` and the file gained a `## Retired rules`
    section, and the skill now takes an explicit add / overwrite / retire
    decision before writing. A pillar that can only add runs out of budget,
    because `AGENTS.md` is paid on every session and gated in CI.
  - `.accelerator-policy-lock.json` ships with the edition: a manifest of
    sha256 digests over the surface the model reads, plus each agent's
    declared model. The session banner prints its short digest beside the
    version, because a version that moves once a release says nothing about
    whether the prompt layer changed.

- Agent `<example>` blocks moved out of `description:` frontmatter into a
  `## Selection examples` body section. An agent's description is loaded into
  the orchestrator's context on every session, spawned or not, and the
  examples were about two thirds of those bytes while teaching the selector
  what the surrounding prose already says. Nothing was deleted - the blocks
  sit verbatim in the body, where a reader still finds them and a session no
  longer pays for them. Measured with cl100k: the agent listing drops from
  5624 t to 1852 t per session, and the edition's whole startup surface
  from 12400 t to 8636 t.

- Spec-driven development: the `sdd` skill and the `/sdd` flow command.
  `/sdd` runs specify -> design -> **checkpoint** -> task breakdown ->
  **checkpoint** -> execute task by task -> tests -> parallel review ->
  verify, spawning roster agents from the main conversation like the other
  flows. What makes it spec-driven rather than a second feature flow is that
  no stage completes without its artifact: the spec and the design land in
  `specs/` and are registered in `specs/MANIFEST.md`, the task breakdown lands
  in `tasks/<TASK-ID>/writing-plans-plan.md` as a checkbox list, and execution
  marks a box and records a `context.py update` per task. The spec carries
  user scenarios, numbered acceptance criteria and an edge-case table; every
  task cites the criteria it serves, each coder capsule carries those criteria
  verbatim rather than just the spec's path, and tests are written per
  criterion. Where a criterion cannot be met as written the flow stops and
  amends the spec instead of quietly redefining done. That citation chain is
  what keeps implementation tied to the spec, and the artifacts are what make a
  run resumable - `/sdd` reads the artifacts, works out which phase is done,
  and continues at the first gap instead of restarting. It drives the
  previously inert `specs/MANIFEST.md` scaffold, adds no new agent, and does
  not integrate: `/finishing-branch` stays a separate step. `/sdd` is listed
  among the sanctioned flow commands in AGENTS.md, so the subagent gate
  treats it as one.

- Opt-in orchestration flows: `/flow-feature` (requirements -> architecture
  -> plan -> checkpoint -> code -> tests -> parallel review -> verify ->
  checkpoint -> finishing-branch) and `/flow-review` (three read-only review
  agents in parallel, one synthesized report). The main conversation is the
  orchestrator: it spawns only roster agents, passes each a bounded
  delegation capsule, and pauses at declared checkpoints. AGENTS.md gains
  the "Orchestration (Flows, SCOPED)" section; SKILL FLOW.md documents the
  flows. Cursor mirrors are generated; Codex keeps its sequential skill
  flow by design.


## 2.0.0 - 2026-08-07

### Shared context runtime

- Adopted the 2026-08-06 shared-core context remediation documented in the
  root changelog: pre-index privacy rejection, canonical task phases, bounded
  2/3/1 capsules, explicit completion, Cursor warming continuity, retrieval
  quality gates, and metadata-only telemetry.
- Adopted the shared builtin hook input hardening and deterministic,
  selected-tool installation inventory/clean-install verification documented in
  the root changelog.

### Fixed

- **Three canonical skills carried `sed` damage that the mirrors did not.**
  `.agents/skills/brainstorming/SKILL.md` and
  `.agents/skills/requirements-analyst/SKILL.md` wrote
  `tasks/TASK-{N}brainstorming-design.md` - a path-separating slash eaten by a
  bulk substitution - and `.agents/skills/review-pr/SKILL.md` described running
  "the systematic systematic-debugger". Repaired in canonical; syncing mirrors
  from canonical would have propagated all three.
- **Two hook timeouts were still in milliseconds.** This edition's
  `bash-validator.sh` and hook wiring were already correct, but
  `.claude/settings.json` kept `8000` on the `UserPromptSubmit` and `Stop` hooks
  where the field is seconds - the two entries missed when the rest of the file
  was converted. Set both to `8`.
- **Stale branch-based wording left over from the pre-monorepo layout** - `AGENTS.md` and `README.md` (intro + Symfony Adaptation Notes) still said things like "this branch is dedicated to Symfony" / "feature/symfony-accelerator branch" / "the Laravel branch", which stopped being accurate once the accelerators were merged into sibling `Laravel/` / `Symfony/` / `PHP Core/` folders in one repo. Reworded to point at the sibling `PHP Core/` and `Laravel/` folders instead of branches, consistent with the root `README.md`.

## 1.3.1 - 2026-07-18

- **Grouped into Symfony folder**


## 1.3.0 - 2026-07-15

### Added

- **Cross-platform memory bank workflow** with one canonical indexed chunk store, native Claude/Cursor command-agent adapters, a Codex-discovered skill, selective retrieval, source verification, lifecycle management, session metadata, privacy controls, deterministic validation, and DOD integration.

## 1.3.0 - 2026-07-15

### Added

- **Symfony 7.4 LTS and Symfony 8.1 accelerator baseline** with consuming-project version detection and PHP 8.2+/8.4+ compatibility guidance.
- **Twelve Symfony specialist workflows** across Claude Code, Cursor, and Codex: API Platform design, architecture-boundary review, console-command implementation, container review, Doctrine migration design, event-subscriber design, fixture/factory generation, Form/Validator design, Messenger design, repository review, voter/security design, and Twig/UX review.
- **Shared architecture policy and clean-code catalog** covering Controller -> Service -> Repository responsibilities, pragmatic SOLID, DTO/Form/Validator boundaries, authorization, Doctrine, Messenger, console, events, API Platform, Twig, and Symfony UX.
- **Complete public onboarding** for prerequisites, native edition layouts, skill flow, security, frontend integration, testing, debugging, performance, research, operations, team adoption, and verification.
- **Senior Symfony clean-code pattern catalog** with paired bad/good examples for layered workflows, pragmatic SOLID, validation, authorization, Doctrine, transactions, Messenger, events, console commands, API Platform, Twig, and tests.
- **Native skill evaluation and benchmarking toolchains for Codex and Cursor** with isolated trigger probes, description optimization, held-out evaluation loops, benchmark aggregation, HTML review, schemas, and deterministic no-credit adapter tests.

### Changed

- **Specialized the framework-neutral branch for Symfony** across root policy, DOD, principles, stabilization guidance, hooks, rules, skills, commands, agents, examples, and edition documentation.
- **Restored Codex's native repository layout**: skills now live in `.agents/skills`; `.codex` contains only project configuration, hooks, and reference documentation.
- **Expanded core Symfony workflows to reference depth** with executable API contract design, layered testing patterns, Twig/Forms/UX implementation, measure-first performance diagnostics, operations documentation, and Symfony Profiler/container/Messenger debugging.
- **Made native handoffs platform-correct**: Codex flow metadata and documentation use discovered skill names, while Cursor retains its supported command aliases without Claude-only wiki syntax or attribution.
- **Expanded Symfony security and operational guidance** for voters, firewalls, CSRF, Serializer exposure, Doctrine parameters, uploads, SSRF, Messenger payloads/retries, Composer advisories, migrations, cache warmup, worker lifecycle, and rollback planning.
- **Expanded frontend guidance** for Twig, Forms, Stimulus, Turbo, Live Components, AssetMapper, Encore compatibility, accessibility, progressive enhancement, validation UX, and browser verification.
- **Established pragmatic SOLID enforcement** across policy and technical workflows: cohesive responsibilities, inward dependencies, narrow real-boundary interfaces, explicit side effects, typed contracts, and resistance to speculative abstraction.
- **Consolidated completed accelerator decisions into root policy, public documentation, and the compact specs manifest**, removing temporary `TASK-001` artifacts and the redundant accelerator implementation spec after review.
- **Removed redundant edition playbooks and unsupported Cursor-style `.codex/rules`**, consolidating enforceable guidance in root policy, living specs, technical skills, Golden Principles, and the shared example catalog.

### Fixed

- Reworked all active PHP examples so good patterns use compatible narrow ports/fakes, deterministic time, non-null authenticated users, bounded queries, explicit runtime type checks, and atomic outbox semantics; bad snippets are now explicitly marked as non-copyable counterexamples.
- Corrected the Codex `systematic-debugger` frontmatter name so repository discovery matches its skill directory.
- Corrected Claude Code hook registration to pass native stdin JSON directly to validators and use the current seconds-based timeout contract.
- Enforced skill-prefixed task/spec Markdown and zero-padded `TASK-001/` directories across all edition hooks using structured JSON path extraction.
- Removed stale phase-map links, cross-product PR attribution, and non-Symfony authorization terminology from native workflows and policy.
- Hardened shell safety hooks with structured JSON command extraction, detection of nested force-push variants such as `--force-with-lease`, and comment-aware fixture safeguards.
- Corrected YAML frontmatter quoting in skill and PHP rule metadata so standard parsers can load every edition reliably.
- Restored the `skill-creator` evaluation resources for Cursor and Codex and replaced Claude-specific subprocess calls, discovery assumptions, schemas, templates, and report copy with isolated native CLI adapters.
- Removed the forced Claude `APP_ENV=local` override and reset loop-detection counters at session start to prevent cross-session false positives.
- Fixed shell safety hooks so blocked patterns beginning with `-` are treated as patterns rather than `grep` options.
- Removed the self-contained `.codex/skills`, `.codex/commands`, and `.codex/agents` duplication that conflicted with Codex repository skill discovery.
- Corrected documentation that previously described Laravel-first history or Codex paths as current Symfony behavior.

## 1.1.0 - 2026-07-09

### Added

- **Cursor edition** - full self-contained `.cursor/` mirror so Cursor users get the accelerator without enabling the opt-in "read `.claude`" setting: 30 skills (`.cursor/skills/`), 28 agents (`.cursor/agents/`), 27 commands converted to Cursor `name`/`description` frontmatter (`.cursor/commands/`), `hooks.json` + scripts with events translated to Cursor's model (`sessionStart`, `beforeShellExecution`, `afterFileEdit`), three always-on/PHP-scoped rules (`.cursor/rules/*.mdc`), DOD/principles/stabilization docs, and a README documenting the double-loading caveat.
- **Codex edition** - Codex-idiomatic layout: 30 skills in `.agents/skills/` (the path Codex discovers), plus `.codex/` holding `config.toml` (enables hooks, MCP placeholder), `hooks.json` + scripts using Codex's Claude-compatible event schema (`SessionStart`, `PreToolUse`, `PostToolUse`), DOD/principles/stabilization docs, and READMEs (`.codex/README.md`, `.agents/README.md`) explaining Codex's model (custom prompts deprecated -> skills; trust requirement).
- **Best-practice depth across skills** - compact, high-signal sections added to:
  - `coder`: modern PHP 8.x idioms, typed exception hierarchy/error handling, PSR-3 logging, configuration, PSR-15 middleware.
  - `test-generator`: AAA, test-double taxonomy, data providers, determinism, coverage targets, mutation testing.
  - `api-designer`: versioning & deprecation, idempotency keys, content negotiation, cursor vs offset pagination.
  - `database-designer`: transaction isolation/deadlocks/locking, expand-contract zero-downtime migrations, soft delete/auditing, correct data types.
  - `architect`: when-not-to-add-a-layer (YAGNI), ports & adapters, concurrency/idempotency, failure/resilience.
  - `code-reviewer`: review-conduct practices (severity labels, actionable feedback, scope).
  - `security-reviewer`: tooling support (`composer audit`, Psalm/PHPStan taint analysis, dangerous-sink grep, secure defaults).
  - `refactorer`: code-smell -> refactoring catalog and the strangler-fig pattern.
  - `writing-plans`: plan-quality practices (safe sequencing, incremental delivery, reversibility).
  - `frontend-design` / `coder-frontend`: frontend best practices and context-aware output escaping + asset hygiene.
  - `documentation-generator`: docs-with-code, runnable examples, Keep a Changelog + SemVer.
  - `using-git-worktrees`: branch and commit hygiene.

### Changed

- **Disambiguated overlapping skills/agents** with explicit "Scope Boundary" sections and tightened descriptions: `coder` (behavior-changing) vs `refactorer` (behavior-preserving) vs `architecture-implementer` (scaffolding); `code-reviewer` (local, broad) vs `security-reviewer` (OWASP-only) vs `review-pr` (remote GitHub PR); `web-design-guidelines` (general UX) vs `wcag-accessibility` (accessibility only).

### Fixed

- **`coder` skill examples corrected** to practice what they preach: replaced the non-existent PSR-7 `->send()` with a proper SAPI emitter; reworked the transactional use case to depend on a repository interface + transaction boundary instead of injecting raw `PDO` into the application layer; added an explicit authorization check in the controller example; flagged illustrative helper imports; and added a `UNIQUE(email)` note to close the check-then-insert race.

## 1.0.0

### Added

- Added claude agents, commands, hooks and skills
- Created a Laravel-first PHP accelerator based on the universal workflow model.
- Added PHP/Laravel onboarding, policy, Definition of Done, and manually authored backend/API/testing guidance.
- Preserved framework-neutral workflow assets, task/spec conventions, product epics, design assets, accessibility guidance, and command-agent-skill flow.
