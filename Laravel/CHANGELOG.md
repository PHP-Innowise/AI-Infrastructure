# Changelog

## Unreleased

### Added

- **Automatic compaction** - `automatic_compaction` and
  `compaction_threshold` (default 5) let the turn-end hook archive terminal
  records in batches rather than one at a time. Within a turn the order is
  complete, promote, then archive, and archived records stay promotable, so a
  capped promotion run never loses the remainder to the archive.
- **Automatic completion on merge** - `automatic_completion` in `runtime.json`
  (enabled by default) lets the turn-end hook complete a task once its branch is
  an ancestor of the default branch, recording the episode and closing the
  handoff. The scan covers every active task, since a merge is observed after
  the branch is left. Merging the default branch into a long-running branch is
  not completion, the default branch never closes itself, and a deleted branch
  is never treated as merged. The outcome states the merge and the ancestry
  check that proved it, not a claim about the work being correct.
- **Automatic promotion to durable memory** - `automatic_promotion` in
  `runtime.json` (enabled by default) lets the turn-end hook promote resolved,
  `verified` knowledge into the Memory Bank unattended, on the same boundary as
  the working-memory flush. The runtime never pretends a human approved it:
  `reviewer` stays null, `review_mode` is `automatic`, the outcome is
  `approved-without-review`, the chunk is tagged `auto-promoted`, and
  `promote-review` refuses to sign an automatic promotion after the fact.
  Eligibility is narrow - only resolved findings/bugs, closed incidents, and
  accepted decisions, never tasks, whose checkpoint progress is not reusable
  knowledge. Sources already promoted are never promoted twice. Set the flag to
  `false` for reviewed promotion.
- **Chunk validity periods** - Memory Bank chunks may carry `valid_from` and
  `valid_to` beside `superseded_by`, which answers a different question: the
  link says what replaced a chunk, the period says when it stopped being true.
  Promotion opens an open-ended period. An active chunk may not sit past its
  `valid_to`, so closing a period removes it from retrieval without deleting
  it, and `archived` finally expresses knowledge that ceased with no successor.
  Both fields are optional, so existing chunks stay valid.
- **Task phase** - a governed task may declare `understanding`, `planning`,
  `execution`, or `finalization`, set with `update --phase` and carried into
  the handoff. Progress records what was touched; the phase records where the
  work stopped. The field is optional so records written before it stay valid,
  and only a task may carry one.
- **Requirement-to-test validation map** - `test-generator` now writes
  `tasks/TASK-NNN/test-generator-validation.md`, one row per requirement with
  its source, the test holding it, and its state. An uncovered requirement is a
  row reading `uncovered`, never an omitted row: a green suite says nothing
  about what it failed to check.
- **`codebase-mapper` skill** - writes `codebase/` documents that describe the
  application source an agent would otherwise have to read: stack,
  architecture, structure, conventions, testing, and concerns. Every claim
  cites a path, secrets are noted by existence and never opened, and each
  document is stamped with the commit and scope it was mapped from so
  retrieval can tell when it has fallen behind.
- **Codebase map freshness** - documents under `codebase/` are indexed as a
  `codebase` kind and checked against the code they describe: commits landed on
  their `mapped_scope` since their `mapped_commit` must stay within
  `codebase_map_max_drift` (default 25). A drifted map is excluded as
  `map-drift`, one without a recorded commit as `map-unverifiable`, and
  `refresh` reports the distance so a stale map is visible before it is
  silently dropped.
- **Description-weighted ranking** - each document is indexed with a `summary`
  drawn from its declared description, weighted above the body in BM25. A skill
  body is procedural prose that reads much alike across skills; the description
  states the topic. Existing databases rebuild themselves on first use.
- **Retrieval relevance** - the index now stems with `porter unicode61`, and
  capsule assembly drops corpus-common terms and requires a document to share
  more than one query term. Without stemming a security question did not
  retrieve the security skill, because its title says "Reviewer" and the query
  said "review". Explicit `search` stays broad. Existing databases rebuild
  themselves on first use; episodes are migrated, not dropped.
- **Zero-command working memory** - the turn-end hook now provisions the task on
  its first flush instead of failing until an operator ran `start`. A goal is
  derived from the branch name and states its own provenance, ticket
  identifiers survive intact, and a task that already exists is never
  overwritten. Buffered turns alone provision nothing, so visiting a branch
  mints no record; only accumulated work does.
- **Per-request memory-layer refresh** - added `context.py refresh`, which
  re-indexes procedural, semantic, and episodic memory in one incremental pass
  and reports each layer as `updated` or `failed`, optionally assembling the
  Task Capsule in the same process. The request hook runs it, so every request
  refreshes all three layers and says so; a failed layer is reported instead of
  silently narrowing the result. Working memory stays out of it: it is written
  by the turn hook, and Project Brain remains its authority in governed mode.
- **Automatic working memory** - added a `UserPromptSubmit` read hook that
  refreshes the index and emits a bounded Task Capsule per request, and a
  `Stop` write hook that buffers each turn's change set and flushes it to the
  authoritative task on a boundary. Reads and writes are split because a
  request has nothing to record yet and the end of a turn does. Both hooks are
  fail-open and time-bounded. Durable memory is unchanged: promotion still
  requires independent human review.
- **Incremental indexing** - `index --incremental` reuses rows whose source
  modification time and size are unchanged, so a no-change refresh performs
  reads only. Secret scanning dominated a full pass, and mirrored skill copies
  that lose the dedup are no longer read at all. Governed `retrieve` now
  refreshes the index itself instead of silently reading a stale one.
- **Ephemeral retrieval manifests** - `retrieve --ephemeral` writes the same
  validated manifest to ignored local state instead of shared Git history, and
  prunes to the most recent 200. Automated retrieval requires it.
- **`--revision auto`** - resolves the current revision inside the mutation
  lock so automated writers perform a real compare-and-swap instead of omitting
  the check.
- **`context.py turn`** - buffers a per-turn working-memory delta from Git
  porcelain metadata alone and flushes a consolidated update on a boundary.
  Sensitive-looking paths and the runtime's own churn are excluded, and a
  per-flush file cap reports what it omits rather than dropping it silently.

- **Combined Project Brain + Local Context Engine architecture** - added
  governed tasks, findings, bugs, incidents, decisions, and events; revision-safe
  handoffs; ownership and conflict metadata; cross-store rollback/compensation;
  compaction; and source/revision-bound promotion proposals with independent
  human review before atomic Memory Bank application.
- **Bounded governed retrieval** - added a dependency-free SQLite FTS5/BM25
  index with privacy, ownership, authority, lifecycle, and source-freshness
  filtering, bounded snippets and token budgets, conflict retention, and
  retrieval manifests that contain metadata rather than source bodies, prompts,
  responses, or hidden reasoning.
- **Authority-aware unified memory workflow** - added `memory` and `checkpoint`
  across Claude, Cursor, and Codex discovery. Source indexing excludes
  Git-ignored files and fails safely on invalid UTF-8 without replacing the
  previous valid index.
- Added bounded Task Capsule retrieval and hybrid fresh-context handoffs for
  complex phase boundaries without adding another memory store or changing
  `memory`, `checkpoint`, or explicit `complete`.
- **Local context engine** - added a dependency-free SQLite FTS5 index for
  policy, specs, active memory, task documents, capability epics, and changelog
  history, plus gitignored summaries of completed tasks. It classifies
  procedural, semantic, episodic, and working context in one local database;
  callers supply task IDs, retrieve bounded per-layer packets, and atomically
  complete a working task into an episode. The shared `memory-bank` skill keeps
  index, search, record, and status compatible without treating local episodes
  as authoritative memory.

### Changed

- **`memory` now runs `refresh`** - the skill reports each memory layer exactly
  as the CLI returned it instead of inferring all three from one `index` exit
  status, and it runs the same command the request hook does, so the skill and
  the hook cannot drift apart. It never passes `--query`: `memory` reports
  layer health and performs no task-aware retrieval.

- **Governed context is now the default** - policy and edition documentation
  define Project Brain as shared active-work authority and SQLite as a
  disposable index plus local binding/cache. They preserve explicit
  `--mode lightweight` for non-authoritative local Working Memory, expose
  `python3 memory-bank/scripts/context.py retrieve QUERY --task-id ID` for
  task-aware retrieval, and state that canonical project sources outrank all
  context.
- **`memory` and `checkpoint` now honor authority** - in governed mode,
  `memory` validates Brain and refreshes the index without creating task
  authority, while `checkpoint` skips local Working Memory and directs a
  revision-checked Brain/handoff update. Branch-derived checkpoints remain
  available only in explicitly configured lightweight mode.
- **Memory Bank ownership narrowed** - the `memory-bank` skill now handles only
  durable retrieval/capture/audit/supersession and application of explicitly
  human-approved promotions; active work and proposals remain in Project Brain.
- **Session hooks remain metadata-only** - startup reporting is limited to mode,
  index health/staleness, active binding count, and validation status, with no
  automatic indexing, retrieval, record printing, or prompt injection.

### Fixed

- **Automatic promotion filtered records in silence.** A decision whose cited
  source had been edited since it was written was held back by the freshness
  check and simply never appeared in durable memory, with nothing said. Every
  eligibility rule now reports the record it blocked and why.
- **Promoted chunks repeated their own title and carried no content.** The
  chunk body was the record's rendered template, so a decision produced a stub
  that stated its heading three times over `Next Steps: None`. Chunks are now
  built from the record's progress note and cited sources, and a record with
  nothing beyond its title is not promoted at all.
- **The relevance filter hid the document that answered the question.**
  Requiring two distinct query terms rewarded documents containing generic
  words and dropped a focused one matching only the single term that mattered:
  a conventions document stating how money is represented lost its slot to
  skills sharing "how" and "project". A term rare in the corpus now qualifies a
  document on its own.
- **Overlapping source patterns aborted the whole index.** Two patterns
  matching one file inserted the document twice and failed the metadata primary
  key, so any future pattern addition that overlapped an existing one would
  have broken indexing entirely. Files are now claimed by the first matching
  pattern.
- **Compaction left promoted Memory Bank chunks citing a path that no longer
  existed.** A chunk records its source record by path; archiving that record
  made the citation dangle, failed Memory Bank validation, and blocked every
  later promotion. Compaction now repoints affected chunks at the archive
  location atomically with the move, and validates the bank before committing
  to it. Present before automation; certain to occur with it.
- **A promotion that failed to apply blocked its source record for good.**
  Deduplication treated any non-rejected promotion as done, so an automatic
  promotion that stalled mid-pipeline was never retried. Stalled automatic
  promotions are now driven forward in place rather than re-proposed.
- **Merge detection completed a task the moment it was provisioned.** A branch
  with no commits of its own is already an ancestor of its target, so ancestry
  alone was not evidence of a merge. The target must also have moved ahead of
  the branch.

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
