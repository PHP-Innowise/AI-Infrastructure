# Changelog

Shared-core history - the Python memory/context core
(`memory-bank/scripts`, `memory-bank/tests`, `project-brain/`), the
tool hooks, and the mirror machinery common to the Laravel, Symfony
and PHP Core editions - is recorded once in the root
[`CHANGELOG.md`](../CHANGELOG.md). This file records only
Laravel-specific changes. The edition's release version is the
`VERSION` file beside this changelog; the SessionStart hook prints
it at the top of every session.

## Unreleased

### Changed

- Memory promotion policy now matches runtime configuration: eligible verified
  terminal records may be applied unattended only when automatic promotion is
  enabled and remain explicitly marked as unreviewed; otherwise an independent
  human review is required.

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
  7405 t to 2104 t per session, and the edition's whole startup surface
  from 14244 t to 8947 t.

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

- **`bash-validator.sh` never blocked anything, and the wiring never reached
  it.** Two independent defects, both silent. The hook scraped the command out
  of the tool-input JSON with `sed` on `"[^"]*"`, so any command containing an
  escaped quote (`php -r "echo 1;" && git push --force`) was truncated past the
  destructive half and passed. Patterns were then handed to `grep -Eqi
  "$PATTERN"` without `--`, so the leading-dash pattern `--no-verify` was parsed
  as a grep option: `git commit --no-verify` printed `grep: unrecognized option`
  to the transcript and exited 0 on every single command. Separately,
  `.claude/settings.json` wired all three PreToolUse/PostToolUse hooks as
  `echo '$TOOL_INPUT' | <script>`, which feeds the hook the literal string
  `$TOOL_INPUT` rather than the payload Claude Code already delivers on stdin,
  and used millisecond `timeout` values (`5000`/`8000`/`10000`) where the field
  is seconds. Adopted the Symfony edition's JSON-decoding extractor
  (jq -> php -> python3), `grep -Eqi --`, and stderr reporting; the wiring is now
  a bare script path with second timeouts. Verified by running both versions
  against the same payloads: `git commit --no-verify` now exits 2 instead of 0.
- **Artisan danger patterns matched prose and read-only searches.** With the
  extractor fixed, the unanchored `migrate.*(:|--)?(fresh|reset|refresh|rollback)`
  would have blocked `grep -rn migrate:refresh docs/` and
  `git commit -m "migrate to the new refresh flow"` - a false positive on
  ordinary work is what gets an enforcement layer switched off. Anchored every
  artisan rule to an actual `artisan` invocation, so `php artisan migrate:fresh`,
  `ddev artisan migrate:rollback`, and `artisan db:wipe` still block while prose
  and searches pass.

## 1.4.3 - 2026-07-18

### Fixed

- **Stale branch-based wording left over from the pre-monorepo layout** - `AGENTS.md`, `README.md` (intro + Adaptation Notes), `.cursor/rules/accelerator-workflow.mdc`, `.cursor/rules/php-standards.mdc`, `GOLDEN-PRINCIPLES.md` (all three editions), `local-context.sh` (all three editions), and the `package-developer` skill (all three editions) still said things like "this branch targets Laravel" / "use the `main` branch" / "feature/laravel-accelerator branch", which stopped being accurate once the accelerators were merged into sibling `Laravel/` / `Symfony/` / `PHP Core/` folders in one repo. Reworded all of these to point at the sibling `PHP Core/` folder instead of a `main` branch, consistent with the root `README.md`. (Legitimate git-branch mentions, like `using-git-worktrees`'s "return work to the main branch" referring to the consuming project's own Git history, were left untouched.)

## 1.4.2 - 2026-07-18

### Added

- **`memory-bank`** - ported the indexed, cross-session, source-verified project-memory system from the Symfony edition, adapted for Laravel conventions instead of copied verbatim:
  - New root `memory-bank/` store: `README.md` (contract/lifecycle), `INDEX.md`, `.memory-counter`, `chunks/MEM-0001-cross-edition-sync.md` (seed chunk documenting the Claude/Cursor/Codex mirroring convention), `templates/chunk.md`, `scripts/validate.py` (dependency-free structural validator), and `tests/test_validate.py`.
  - New `memory-bank` skill/command/agent across `.claude/`, `.cursor/`, and `.agents/skills` (Codex has no command/agent layer, per convention). The Laravel-specific memory categories replace Symfony's Controller -> Service -> Repository/Messenger wording with Controller -> Action/Service -> Eloquent model boundaries and queue-worker operational lessons.
  - Wired into `AGENTS.md` (hierarchy of sources of truth, file naming, agent behavior, and a new "Memory Bank" policy section), `SKILL FLOW.md` (utility row + shortcut + Context Handoff line), `README.md` (What This Is, directory structure, Quick Start table, and a new "Memory Bank" section), `.cursor/rules/accelerator-workflow.mdc`, and each edition's `README.md` (skill count bumped to 40).
  - `local-context.sh` (all three editions) now reports memory-bank chunk counts at session start via `scripts/validate.py --summary`, and lists `memory-bank/` in the project structure scan. Also corrected the Codex hook's structure-scan marker from a stray `.claude` to `.agents .codex`.
  - `.gitignore` updated to ignore `memory-bank/local/`, `.idea/`, and Python tooling caches, matching the Symfony edition.
- All 14 validator tests (including the cross-edition hook-integration test) pass against the new store.

## 1.4.1 - 2026-07-18

### Changed

- **Added**:
  - **`api-designer`:** noted single-action invokable controllers as the natural next step once an action endpoint (`accept`/`cancel`/`publish`) outgrows a method on the resource controller; added `toDto()`-on-a-typed-value-object guidance for Form Requests once a payload has enough fields that an untyped array gets hard to follow (cross-referenced to `architect`'s existing YAGNI stance on DTOs so it doesn't contradict "don't reach for `spatie/laravel-data` by default"); restructured the pagination section so `cursorPaginate()` is the explicit default and `paginate()` is presented as a deliberate exception (client-facing numbered pager or a required total count) rather than an easy-to-reach-for alternative for "large tables" — folding the previously separate "Cursor vs Offset Pagination" section into it so the guidance can't be skimmed past.
  - **`architect`:** added a "Repositories" section clarifying that Eloquent is already the persistence layer (Active Record), so a Repository is not the default — and naming the concrete cases (a real backend-swap plan, a complex reused query, or a test seam) where one earns its keep, with a matching bullet in "When NOT To Add A Layer".
  - **`architecture-implementer`:** the scaffolded model's `$fillable` example intentionally stays property-based rather than adding a native `array` type, because `Model::$fillable` is declared untyped in Eloquent's base class and PHP's invariant property-typing rules turn a typed override into a fatal error; added a note pointing to Laravel 13's optional `#[Fillable(...)]` attribute (PHP 8.3+, `eloquent` skill) as an alternative for projects that have already standardized on attribute-based model configuration, without rewriting the default example in a way that would break on Laravel 12.
- **Removed a repeated "for framework-agnostic native PHP, use the `main` branch instead" sentence** duplicated across 17 skills (`architect`, `auth-scaffolding`, `brainstorming`, `caching`, `code-reviewer`, `coder`, `console-scheduler`, `dependency-manager`, `eloquent`, `events-notifications`, `filament`, `file-storage`, `performance-optimization`, `queues-jobs`, `refactorer`, `security-reviewer`, `test-generator`, `verify`) — the branch-strategy pointer is stated once, authoritatively, in `AGENTS.md`/`README.md`/`GOLDEN-PRINCIPLES.md`, so repeating it as boilerplate in every skill was noise flagged in review.
- Re-synced all of the above across `.claude/`, `.cursor/`, and `.agents/`/`.codex/`.
- **Grouped into laravel folder**

## 1.4.0 - 2026-07-17

### Added

- **8 new Laravel-specific skills** (+ matching command + agent each, across all three editions): the accelerator now covers the core Laravel building blocks that sat below `coder`'s general-purpose level in depth.
  - **`eloquent`** - deep model-layer patterns once the schema exists: polymorphic relationships, modern `Attribute::make()` accessors/mutators, custom cast classes vs. backed-enum casts, local/global query scopes, model events/Observers, mass-assignment protection, advanced eager loading (`withCount()`/`withExists()`), and large-dataset iteration (`chunk()`/`cursor()`).
  - **`queues-jobs`** - queued Job class anatomy, job middleware (`WithoutOverlapping`, `RateLimited`, `ThrottlesExceptions`), unique jobs (`ShouldBeUnique`), batching vs. chaining, failed-job handling/retries, and Horizon supervisor configuration.
  - **`events-notifications`** - Events/Listeners, model Observers, and Notifications (mail/database/broadcast/Slack) with Mailables, for decoupled side effects and multi-channel user communication.
  - **`auth-scaffolding`** - web/session auth starter kits (Breeze/Jetstream/Fortify), multi-guard configuration, and Policy/Gate authorization; explicitly scoped away from token-based API auth, which stays with `api-designer`.
  - **`caching`** - `Cache::` facade patterns, stampede prevention, driver-specific tagging caveats, model-level caching, and invalidation-on-write correctness.
  - **`console-scheduler`** - custom Artisan command signatures/output and task scheduling (overlap prevention, multi-server safety, failure handling).
  - **`file-storage`** - the `Storage` facade/disk abstraction, secure upload validation, signed/temporary URLs, and streaming large files; hands the security audit pass to `security-reviewer` rather than duplicating it.
  - **`package-developer`** - Composer package skeleton and auto-discovery, Service Provider structure, config/migration/view publishing, and Orchestra Testbench testing; flagged as rarely needed and distinct from `dependency-manager` (consuming vs. building a package).
- **Wired all 8 new skills** into `SKILL FLOW.md` (main flow, shortcuts, and phase map), `README.md`'s Quick Start command table, and targeted cross-references in `architect`, `database-designer`, `api-designer`, `security-reviewer`, `dependency-manager`, `coder`, and `performance-optimization` so the existing skills point to the new, deeper ones instead of duplicating their content.

### Fixed

- **Corrected drift between editions** introduced by the 1.3.0 round: `code-reviewer`/`reflect`/`release`/`verify`'s edition-specific path substitutions were untouched (correct), but `SKILL FLOW.md` and the seven cross-referenced skills above had fallen out of sync between `.claude/`, `.cursor/`, and `.agents/skills/` - re-synced across all three.
- **Fixed a self-referential path bug** in `.cursor/README.md`'s "Keeping the two copies in sync" section, which incorrectly described `.cursor/` as generated from itself instead of from `.claude/`.
- **Bumped stale skill-count references** (31 -> 39) in `.cursor/README.md`, `.agents/README.md`, and `.codex/README.md`.

## 1.3.0 - 2026-07-10

### Added

- **New `filament` skill (+ command + agent)** across all three editions - builds Filament admin panels (Resources, Schemas for Forms/Infolists, Tables, Relation Managers, unified `Filament\Actions`, custom Pages, and Widgets) backed by Eloquent models and Policy-based authorization, including a Livewire-based testing pattern. Wired into `SKILL FLOW.md`, `README.md`'s command table, and `architect`'s admin-screens guidance. Filament is now the recommended default for new Laravel admin panels given its decisive lead over Nova in ecosystem adoption.
- **`laravel/boost` recommended** in `dependency-manager` and `README.md` prerequisites - the official first-party MCP dev-dependency that gives AI coding agents live access to a project's routes, Eloquent schema, config, Tinker REPL, logs, and version-pinned docs, directly reducing hallucinated APIs for this accelerator's own workflows.
- **Laravel Octane coverage** in `performance-optimization` - when to adopt it, the `singleton()` vs `scoped()` cross-request state-leak gotcha with a corrected code example, and an adoption/soak-test checklist.
- **Parallel testing and Pest 4 browser testing** in `test-generator`/`verify`/`DOD.md` - `php artisan test --parallel` (`brianium/paratest`) guidance, and Pest 4's built-in Playwright-powered browser testing (now recommended by Laravel's own docs over Dusk for new projects), with an explicit contrast against the existing `browser-verify` skill's manual/exploratory role.
- **Deepened `api-designer`** with a Sanctum vs Passport vs stateless-JWT authentication decision table.
- **Deepened `architect`** with multi-tenancy guidance (single-database scoping vs `stancl/tenancy` multi-database isolation), real-time/broadcasting guidance (Laravel Reverb + `ShouldBroadcast` as the default over third-party WebSocket services), and a "Deployment Considerations" note (cache invalidation, maintenance mode, queue-payload compatibility across major-version upgrades).
- **Deployment-readiness checklist items** added to `release` and `DOD.md`'s Full tier (queue-drain/compatibility check and cache invalidation check before shipping a Laravel major-version bump).
- **Cashier webhook-idempotency note** and **PHPStan/Larastan baseline-and-ratchet strategy note** (plus a known generics-invariance caveat) added to `dependency-manager` and `code-reviewer` respectively.
- **Livewire v4 / Volt currency note** in `coder-frontend` - prefer Livewire v4's native single-file components over adding `livewire/volt` on new projects; Volt remains valid on Livewire v3 projects.
- **Forms UX and empty/error-state depth** added to `frontend-design`'s best-practices section (focus management on validation failure, distinguishing "no data yet" from "no results matched" empty states, retry actions on error).

### Changed

- **Bumped the stated Laravel/PHP baseline from "Laravel 11/12, PHP 8.2+" to "Laravel 12 or 13, PHP 8.2+ (8.3+ required for Laravel 13)"** across `README.md` and every skill's boilerplate targeting line (`coder`, `architect`, `refactorer`, `dependency-manager`, and the new `filament` skill), reflecting Laravel 13's March 2026 release as the current stable version.
- **Re-scoped `web-design-guidelines`** as a supplementary, periodic "fetch the latest external UX checklist" utility (its content is a live external fetch with no embedded, Laravel-specific rules) rather than a required step in the main flow, now that `frontend-design`'s own best-practices section is self-sufficient for day-to-day work.
- **Re-mirrored `.cursor/` and `.agents/skills` + `.codex/`** from the updated `.claude/` content to keep all three editions in parity (32 skill entries, 30 agents, 29 commands each), including the new `filament` skill/agent/command and the version-baseline bump in `.cursor/rules/php-standards.mdc`.

This round of changes was informed by a dedicated research pass into the current (2026) Laravel ecosystem to identify gaps, outdated assumptions, and redundant skills in the accelerator.

## 1.2.0 - 2026-07-10

### Changed

- **Reverted the accelerator to Laravel-first on `feature/laravel-accelerator`** - the framework-agnostic native-PHP base introduced in 1.1.0 now lives exclusively on `main`; this branch re-specializes every policy file, hook, skill, command, and agent for Laravel (PHP 8.2+, Laravel 11/12, Composer, Artisan, Eloquent).
- **Core policy and config**: `AGENTS.md`, `.claude/DOD.md`, `.claude/GOLDEN-PRINCIPLES.md`, and `.claude/STABILIZATION.md` rewritten around Laravel conventions - Form Request validation, Policy/Gate authorization, Eloquent persistence and eager loading, Artisan migrations, Pint/Larastan/Pest tooling. `.claude/settings.json` permissions and allowed `WebFetch` domains updated for `artisan`, `pint`, and the Laravel package ecosystem (laravel.com, livewire, inertiajs, spatie, packagist, pestphp).
- **Hooks**: `local-context.sh` now detects Laravel (`artisan --version`), Livewire/Inertia frontend stacks, and flags unexpected Symfony coexistence; `bash-validator.sh` blocks destructive `artisan migrate:*` resets/rollbacks, `db:wipe`, forced `db:seed --force`, and `model:prune` in addition to the existing destructive-command list.
- **All 30 skills (+ matching commands/agents) rewritten for Laravel**, across five clusters:
  - Backend: `coder`, `architect`, `architecture-implementer`, `api-designer`, `database-designer`, `refactorer`, `dependency-manager`.
  - Quality: `test-generator`, `code-reviewer`, `security-reviewer`, `performance-optimization`, `systematic-debugger`, `verify`.
  - Frontend: `frontend-design`, `coder-frontend`, `browser-verify` - retargeted to Blade, Livewire, and Inertia.
  - Planning/docs: `brainstorming`, `requirements-analyst`, `writing-plans`, `council`, `researcher`, `documentation-generator`.
  - Utility: `release`, `finishing-branch`, `using-git-worktrees`, `reflect`, `review-pr`.
- **Root and example docs** (`README.md`, `spec-desc.md`, `examples/*.md`, `Task/designs/DESIGN_TOKENS.md`) updated to Laravel terminology, verification commands, and workflow examples.
- **`.cursor/` and `.agents/skills` + `.codex/` editions regenerated** from the updated `.claude/` content to keep all three tool mirrors in parity (same skill/agent/command counts, frontmatter, and hook behavior), including `.cursor/rules/*.mdc` and both editions' README/DOD/GOLDEN-PRINCIPLES/STABILIZATION docs.

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
