# PHP Frameworks Reference

Detection signals and baseline skill scaffolding per PHP framework. Used by `skill-forge` to shape universal skills to the target's real framework. Always subordinate to actual evidence in the target - never impose a framework's conventions on a project that does not use it.

## Laravel

- **Signals:** `laravel/framework` in `composer.json`; `artisan`; `bootstrap/app.php`; `app/`, `routes/`, `config/`, `database/migrations/`.
- **Persistence:** Eloquent ORM (`app/Models`), migrations, factories, seeders.
- **HTTP:** routes/controllers, Form Request validation, middleware, API Resources.
- **Async:** queued jobs, events/listeners, scheduler (`app/Console/Kernel.php` or `bootstrap/app.php` scheduling).
- **Testing:** PHPUnit or Pest under `tests/`; `php artisan test`.
- **Tooling to reference if present:** Pint (`pint.json`), Larastan (`phpstan.neon`), Rector.
- **Baseline skills:** coding (routes/controllers/Eloquent/Actions), testing (feature+unit), code-review, security-review (auth guards, mass-assignment, validation), performance (N+1, eager loading, cache), release, debugging.

## Symfony

- **Signals:** `symfony/framework-bundle`; `bin/console`; `config/bundles.php`; `src/`, `config/`, `migrations/`.
- **Persistence:** Doctrine ORM (entities, repositories), Doctrine migrations.
- **HTTP:** controllers, routing (attributes/YAML), the Form component, Validator constraints, Security voters/guards.
- **Async:** Messenger (message buses, handlers, transports), event subscribers.
- **Testing:** PHPUnit under `tests/`; `bin/phpunit` or `php bin/console`.
- **Tooling to reference if present:** PHP-CS-Fixer, PHPStan, Psalm, Rector.
- **Baseline skills:** coding (controllers/services/Doctrine), testing, code-review, security-review (voters, CSRF, validation), performance (Doctrine hydration, cache), release, debugging.

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
