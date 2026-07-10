# Changelog

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
