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

## Design & Interaction Skills (always generated, evidence-shaped)

Alongside the architecture skill (section 3), generate these three for every target - they carry real framework content (per `references/php-frameworks.md` above) but, unlike the framework-specialty catalog, are not conditional; every PHP project makes these decisions somewhere:

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
