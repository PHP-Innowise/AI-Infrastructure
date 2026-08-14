# PHP Frameworks Reference

Detection signals and baseline skill scaffolding per PHP framework. Used by `skill-forge` to shape universal skills to the target's real framework. Always subordinate to actual evidence in the target - never impose a framework's conventions on a project that does not use it.

## Laravel

- **Signals:** `laravel/framework` in `composer.json`; `artisan`; `bootstrap/app.php`; `app/`, `routes/`, `config/`, `database/migrations/`.
- **Persistence:** Eloquent ORM (`app/Models`), migrations, factories, seeders.
- **HTTP:** routes/controllers, Form Request validation, middleware, API Resources.
- **Async:** queued jobs, events/listeners, scheduler (`app/Console/Kernel.php` or `bootstrap/app.php` scheduling).
- **Testing:** PHPUnit or Pest under `tests/`; `php artisan test`.
- **Tooling to reference if present:** Pint (`pint.json`), Larastan (`phpstan.neon`), Rector.
- **Baseline skills:** coding (routes/controllers/Eloquent/Actions), testing (feature+unit), code-review, security-review (auth guards, mass-assignment, validation), performance (N+1, eager loading; cache as a lever only - defer to `caching-strategy` if generated), release, debugging (defer to `systematic-debugger` for methodology).

## Symfony

- **Signals:** `symfony/framework-bundle`; `bin/console`; `config/bundles.php`; `src/`, `config/`, `migrations/`.
- **Persistence:** Doctrine ORM (entities, repositories), Doctrine migrations.
- **HTTP:** controllers, routing (attributes/YAML), the Form component, Validator constraints, Security voters/guards.
- **Async:** Messenger (message buses, handlers, transports), event subscribers.
- **Testing:** PHPUnit under `tests/`; `bin/phpunit` or `php bin/console`.
- **Tooling to reference if present:** PHP-CS-Fixer, PHPStan, Psalm, Rector.
- **Baseline skills:** coding (controllers/services/Doctrine), testing, code-review, security-review (voters, CSRF, validation), performance (Doctrine hydration; cache as a lever only - defer to `caching-strategy` if generated), release, debugging (defer to `systematic-debugger` for methodology).

## Slim / Laminas / Mezzio / Micro-frameworks

- **Signals:** `slim/slim`, `laminas/laminas-mvc` or `mezzio/mezzio`; PSR-7/PSR-15 middleware pipelines; `public/index.php` front controller.
- **Persistence:** varies - Doctrine, Eloquent standalone, or PDO. Detect from packages.
- **HTTP:** PSR-15 middleware, route definitions, DI container config.
- **Baseline skills:** coding (middleware/handlers), testing, code-review, security-review, performance, release, debugging - scoped to the actual PSR components in use.

## CodeIgniter / Yii / CakePHP

- **Signals:** `codeigniter4/framework`, `yiisoft/yii2`, `cakephp/cakephp`; their respective entry points and directory conventions.
- **Baseline skills:** same universal set, adapted to the framework's own MVC/ORM idioms as evidenced in the target.

## Plain PHP (no framework)

- **Signals:** `composer.json` with a PSR-4 autoload map but no framework package; a hand-rolled front controller in `public/`.
- **Persistence/HTTP:** whatever libraries are present (PDO, Guzzle, a router package). Detect and scope skills to those.
- **Baseline skills:** coding, testing, code-review, security-review, performance, release, debugging - grounded strictly in the libraries actually used, with no framework assumptions.

## Universal Skill Shape (all frameworks)

Every generated universal skill must: name the target's REAL tools (test runner, static analyzer, formatter) from the profile; cite the config files that prove them; and avoid recommending a tool the target does not use.

### `debugging`'s Scope (split from the process skill `systematic-debugger` - do not duplicate)

`debugging` owns only the **"where to look in THIS target"** half: its real error tracker/APM (e.g. Sentry, cited from section 4/8), its real log locations and format, and any real in-app debugging tool it ships (Xdebug config, Laravel's `ray()`/`dd()`, Symfony's `VarDumper`/profiler, or nothing beyond plain logs if that's all the evidence shows). It explicitly assumes the reader already knows the root-cause-first investigative discipline and cross-references `references/php-process-skills.md`'s `systematic-debugger` for it, rather than re-stating that methodology. Never let `debugging` re-explain "reproduce, isolate, confirm the cause" - that content belongs solely to `systematic-debugger`.

### `performance`'s Scope re: caching (split from the specialty skill `caching-strategy` - do not duplicate)

`performance` owns the **measure-first workflow across every hot path** (queries/N+1, eager loading, queue throughput, outbound HTTP calls, and cache AS ONE LEVER among these) - baseline, profile, fix the top hotspot, re-measure, lock in a budget. When profile section 3.1 confirms an in-app caching strategy is present (so `caching-strategy` is also being generated), `performance` MUST NOT re-derive cache-correctness content (invalidation-on-write, stampede prevention, tag design) - it names caching only as "a hot path to consider" and links to `caching-strategy` for that depth. Only when `caching-strategy` is NOT generated (no such signal in section 3.1) may `performance` cover baseline cache-as-a-lever guidance itself, since nothing else will.

### `api-designer` / `database-designer` cross-references

See the scope notes inline in "Design & Interaction Skills" above - `api-designer` defers to `api-platform-design` when a declarative API framework is the target's primary mechanism, and `database-designer` defers to `orm-patterns` for ORM usage patterns.

## Design & Interaction Skills (evidence-gated)

These are candidates, not a baseline. Generate one only when its selection trigger is evidenced:

- `architecture-implementer` - scaffolds the skeleton for a new feature (classes/interfaces/DI wiring, or the framework's own code-gen CLI where one exists e.g. `artisan make:*`/`make:*` for Symfony) matching the detected architecture pattern from section 3, leaving business logic as a clearly marked TODO.
- `api-designer` - designs the target's real HTTP API contract shape (routes, request validation, response/DTO shape, auth, pagination, error format) using the target's actual framework conventions (Form Requests + API Resources for Laravel; request DTOs/Forms + Serializer for Symfony; PSR-7 + hand-rolled validators for plain PHP/micro-frameworks). **Scope note (cross-references `api-platform-design`):** if profile section 3.1 confirms a declarative API resource framework (e.g. API Platform) as the target's primary or sole API mechanism, `api-designer` narrows to whatever hand-rolled routes/controllers remain (if any) and explicitly defers all resource/operation/metadata design to `api-platform-design` (`php-specialty-skills.md`) rather than re-deriving declarative-resource guidance here.
- `database-designer` - designs the **schema itself**: tables/entities, columns, keys, indexes, constraints, normalization, and which migrations to author, matching the target's real persistence layer (Eloquent migrations; Doctrine ORM migrations; plain PDO with hand-rolled integrity checks if no ORM is present). **Scope note (cross-references `orm-patterns`):** it does NOT cover how application code queries/manipulates that schema once designed (relationships-as-used-in-code, accessor/mutator/cast patterns, query scopes, eager-loading/N+1 avoidance) - that usage-pattern content belongs solely to the specialty skill `orm-patterns` (`php-specialty-skills.md`) when an ORM is confirmed, and `database-designer` MUST link to it rather than duplicate it.

## Frontend Skills (conditional on a detected rendering/templating/asset layer)

Generate this group only when the profile records evidence of a server-rendered templating layer (Blade/Twig/plain PHP templates) and/or a frontend asset build (`package.json` with a bundler, a `resources/js`-style directory, or similar) - i.e. the target is not a pure API/CLI/library with no UI surface:

- `frontend-design` - chooses/designs the rendering approach (templating engine, CSS strategy, progressive enhancement, accessibility needs) before implementation, matching whatever the target already uses (Blade/Livewire/Inertia; Twig/Symfony UX/Stimulus/Turbo; plain templates + vanilla JS).
- `coder-frontend` - implements the frontend changes for a `frontend-design` decision, in the target's real templating/asset stack.
- `wcag-accessibility` - WCAG 2.2 AA reference checklist applied to the target's real markup/templates.
- `web-design-guidelines` - supplementary UX/interaction-polish checklist (fetched from the public Web Interface Guidelines reference), not required but useful for user-facing UI work.
- `browser-verify` - visually verifies UI changes against the target's real dev-server/build setup, supplementing (not replacing) automated tests.

If no rendering/asset layer is detected (a pure API backend, a CLI tool, or a library), skip this entire group and say so explicitly in the profile - do not generate frontend skills for a project with no UI surface.

## Enforceable Candidate Contracts

For every selected skill, the plan must replace examples with target-relative evidence and include positive/negative triggers, owned/excluded scope, decision points, procedure roles, verification, output, failure handling, writes, and sibling boundaries.

### Design & interaction

#### `architecture-implementer`
- **Select/evidence:** confirmed architecture boundaries plus an evidenced feature-scaffolding convention or repeated feature skeleton.
- **Own/exclude:** scaffold one feature through existing layers and DI; exclude architecture redesign and business-rule invention.
- **Procedure:** identify owning module/layers; select existing exemplar; create interfaces/classes/wiring in dependency order; leave explicit business TODOs; check dependency direction.
- **Verify/output:** autoload/container/static checks pass; output scaffold paths, wiring, TODOs, and commands. Stop on ambiguous ownership.
- **Sibling/negative:** architecture skill decides boundaries; `coding` implements behavior. Good: mirror a cited module skeleton. Bad: create generic Controller/Service/Repository folders.

#### `api-designer`
- **Select/evidence:** confirmed HTTP/API routes and at least one canonical request/response/auth/error convention.
- **Own/exclude:** API contract, validation, auth, pagination, idempotency, errors; exclude persistence schema and declarative resources owned by `api-platform-design`.
- **Procedure:** identify consumers/authority; inspect endpoint exemplars; define operation/schema/status/errors; map permissions and transitions; define compatibility and contract tests.
- **Verify/output:** contract is internally consistent and testable; output endpoint/DTO/error/test design. Stop when permission or source authority is unknown.
- **Sibling/negative:** defer declarative metadata to `api-platform-design`, schema to `database-designer`. Bad: “Use REST best practices.”

#### `database-designer`
- **Select/evidence:** confirmed persistent store plus schema/migration/entity evidence and a requested data-model change.
- **Own/exclude:** tables/collections, keys, constraints, indexes, normalization, migration intent; exclude ORM query patterns and rollout mechanics.
- **Procedure:** derive invariants/access paths; model identifiers/relations; choose constraints/indexes; assess data compatibility; specify migrations and integrity tests.
- **Verify/output:** schema tool/dry-run and representative query/constraint checks; output model, migration sequence, risks, tests. Stop rather than infer missing invariants.
- **Sibling/negative:** `orm-patterns` owns application usage; `migration-safety` owns rollout. Bad: add indexes “for performance” without access evidence.

### Universal PHP

#### `coding`
- **Select/evidence:** recurring PHP implementation conventions are evidenced in source plus formatter/static-analysis configuration.
- **Own/exclude:** implement behavior in existing patterns; exclude design discovery, dependency selection, and release.
- **Procedure:** load closest exemplar and contract; trace boundaries/types/errors; implement smallest change; apply target formatter/static analyzer; add/update focused tests.
- **Verify/output:** evidenced format, static analysis, and tests pass; output files, behavior, commands/results, residual risks. Never assume framework defaults.
- **Sibling/negative:** `architecture-implementer` scaffolds; `refactorer` preserves behavior. Bad: generic SOLID boilerplate unrelated to target code.

#### `testing`
- **Select/evidence:** runnable test tooling/config and target suite conventions exist.
- **Own/exclude:** choose test level, fixtures, assertions, and regression coverage; exclude root-cause methodology and production monitoring.
- **Procedure:** map changed rule to observable outcome; choose unit/integration/feature level from target patterns; construct deterministic data; cover allowed/denied/failure paths; run narrow then required broad checks.
- **Verify/output:** prove fail-before/pass-after when feasible; output tests/plan and commands/results. Report unavailable tooling without inventing commands.
- **Sibling/negative:** `systematic-debugger` finds causes; `test-data-factories` owns factory design. Bad: assertions that only check HTTP 200.

#### `code-review`
- **Select/evidence:** a local diff/change set and evidenced project rules/DoD exist.
- **Own/exclude:** correctness, maintainability, tests, contracts, and project conventions in the supplied diff; exclude remote PR administration.
- **Procedure:** establish base/scope; inspect full diff and callers; trace affected invariants/permissions/failures; validate suspected findings; rank by consequence.
- **Verify/output:** each finding cites location, impact, and remediation; output prioritized findings and test gaps. Say no findings when none are substantiated.
- **Sibling/negative:** `review-pr` owns remote PR context; `security-review` owns threat-focused depth. Bad: style-only checklist with no diff evidence.

#### `security-review`
- **Select/evidence:** change touches evidenced trust boundaries, auth, sensitive data, inputs, files, commands, or external integrations.
- **Own/exclude:** threat paths, authorization, validation, secret/data handling, injection, abuse and audit integrity; exclude general code quality.
- **Procedure:** map assets/actors/entry points; trace untrusted data and object ownership; test deny paths; inspect logging/storage/transport; validate mitigations and residual risk.
- **Verify/output:** run available security/static tests and concrete exploit/deny cases; output findings with evidence, severity rationale, and fixes. Do not claim compliance.
- **Sibling/negative:** integration skill owns provider mechanics; domain review owns business invariants. Bad: generic OWASP list detached from changed paths.

#### `performance`
- **Select/evidence:** measurable hot path, budget/SLO, profiling data, or evidenced performance regression exists.
- **Own/exclude:** measure-first cross-layer optimization and budgets; exclude cache correctness when `caching-strategy` exists.
- **Procedure:** define workload/metric; capture baseline; profile; rank bottlenecks; change one high-impact cause; remeasure; add budget/regression check.
- **Verify/output:** comparable before/after data; output profile evidence, change, results, and trade-offs. Stop if no representative measurement is possible.
- **Sibling/negative:** `caching-strategy` owns invalidation/keys; `repository-review` owns data-access semantics. Bad: “Add caching and indexes.”

#### `release`
- **Select/evidence:** confirmed CI/CD or documented release/deployment procedure and versioning/artifact conventions.
- **Own/exclude:** release readiness, artifact/version/changelog, deployment sequence, rollback and post-release checks; exclude branch finishing and feature redesign.
- **Procedure:** identify release type/target; run evidenced gates; assess migrations/config compatibility; build/sign/version as configured; execute approved deployment; verify health; invoke rollback criteria.
- **Verify/output:** CI/artifact/deployment and smoke evidence; output version, gates, deployment/rollback status. Do not invent or execute absent deployment commands.
- **Sibling/negative:** `finishing-branch` owns branch handoff; `migration-safety` owns migration rollout details. Bad: universal “composer install --no-dev” instructions.

#### `debugging`
- **Select/evidence:** target-specific logs, profiler, error tracker, trace correlation, or debug configuration is confirmed.
- **Own/exclude:** where and how to collect diagnostics in this target; exclude reproduce/isolate/root-cause methodology.
- **Procedure:** map symptom to evidenced telemetry sources; select safe environment/time/request/task correlation; collect sanitized diagnostics; interpret tool-specific fields; hand evidence to `systematic-debugger`.
- **Verify/output:** confirm signal freshness/correlation and redact sensitive data; output locations/queries, sanitized evidence, and gaps. Report absent telemetry honestly.
- **Sibling/negative:** `systematic-debugger` exclusively owns investigative method. Bad: repeat “reproduce, hypothesize, isolate.”

### Frontend

#### `frontend-design`
- **Select/evidence:** UI verdict applies and real template/component/CSS/JS conventions exist.
- **Own/exclude:** rendering/interaction/accessibility design before implementation; exclude coding and visual test execution.
- **Procedure:** identify user task/states; inspect existing components/tokens; choose server/progressive/client boundary; specify responsive, keyboard, error/loading/empty behavior; hand off design.
- **Verify/output:** design covers WCAG-relevant states and target constraints; output component/interaction specification. Do not introduce a new frontend stack without approval.
- **Sibling/negative:** `coder-frontend` implements; `wcag-accessibility` audits. Bad: generic dashboard mood board.

#### `coder-frontend`
- **Select/evidence:** an approved UI design plus evidenced template/asset stack.
- **Own/exclude:** implement markup/styles/behavior in existing stack; exclude product redesign and backend contract invention.
- **Procedure:** map design to existing components; implement semantic HTML first; wire progressive behavior; handle all states; add automated tests; build assets.
- **Verify/output:** target build/tests pass and keyboard/reflow checks complete; output files and results. Stop on missing API/design decisions.
- **Sibling/negative:** `frontend-design` owns decisions; `browser-verify` owns visual runtime evidence. Bad: replace Twig/Blade with a new SPA.

#### `wcag-accessibility`
- **Select/evidence:** UI surface exists and the task creates/reviews rendered interaction.
- **Own/exclude:** WCAG 2.2 AA semantic, keyboard, focus, forms, contrast, media, motion, reflow checks; exclude subjective visual polish.
- **Procedure:** inspect rendered HTML; test semantics/name-role-value; keyboard/focus; form errors; contrast/zoom/reflow/motion; map failures to criteria and fixes.
- **Verify/output:** automated checks plus manual keyboard/zoom evidence; output criterion-tagged findings/fixes. Never use ARIA to replace native semantics.
- **Sibling/negative:** `web-design-guidelines` owns UX polish; `browser-verify` captures visual behavior. Bad: “add aria-label everywhere.”

#### `web-design-guidelines`
- **Select/evidence:** user-facing UI exists and UX/interaction-polish review is requested.
- **Own/exclude:** hierarchy, consistency, feedback, error prevention, responsive interaction, and content clarity; exclude normative WCAG claims.
- **Procedure:** load target design language; inspect primary tasks/states; assess hierarchy/actions/feedback/recovery; compare sibling screens; propose bounded improvements.
- **Verify/output:** findings cite rendered states and target patterns; output prioritized UX findings. Mark external guidelines as advisory.
- **Sibling/negative:** `wcag-accessibility` owns conformance. Bad: generic “use more whitespace.”

#### `browser-verify`
- **Select/evidence:** a runnable UI/dev-server route and expected changed behavior are known.
- **Own/exclude:** browser-based visual/interaction verification; exclude implementation and replacing automated tests.
- **Procedure:** start/use evidenced environment; navigate critical paths; inspect desktop/narrow widths; exercise keyboard/forms/errors; capture screenshots/console/network evidence; compare to acceptance.
- **Verify/output:** record routes, viewport, actions, artifacts, and failures; output verification report/screenshots. Stop for auth/captcha/destructive blockers.
- **Sibling/negative:** `testing` owns automated regression; `coder-frontend` fixes code. Bad: declare success from a static screenshot alone.
