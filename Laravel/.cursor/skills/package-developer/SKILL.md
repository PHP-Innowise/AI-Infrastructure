---
name: package-developer
description: Build and maintain a reusable Composer/Laravel package - service provider structure, config/migration/view publishing, and testing with Orchestra Testbench. Use only when the deliverable is a standalone package, not an application feature.
phase: execution
flow-next: test-generator
flow-alternatives: [documentation-generator, release]
related: [dependency-manager, architect, release]
---

# Package Developer

## Overview

Build and maintain a reusable Composer package consumed by other Laravel applications: package skeleton, Service Provider structure, publishable config/migrations/views, and package-level testing with Orchestra Testbench.

**This skill is rarely needed.** Most work in this accelerator is an application feature living inside a single Laravel app — that is `coder`/`coder-frontend`/`architecture-implementer` territory, not this one. Reach for `package-developer` only when the actual deliverable is a standalone, versioned, Composer-installable package meant to be reused across multiple applications (an internal shared library, an open-source package, an extraction of duplicated logic into its own repo). If you're unsure which one applies: if the code will only ever live in `app/` of one project, this is the wrong skill.

This is the `Laravel/` accelerator folder (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). Verified current versions (2026): `orchestra/testbench` v10.x pairs with Laravel 12, v11.x with Laravel 13; `spatie/laravel-package-tools` 1.93.x supports Laravel 10-13. Confirm the exact Testbench major against the package's target Laravel version before scaffolding tests, since Testbench majors track Laravel majors one-to-one.

## Scope Boundary

- `dependency-manager` decides whether to **pull in** a third-party package into an application (vetting, version constraints, `composer audit`). `package-developer` is for **building** one from scratch or maintaining an existing one.
- `architect` decides *whether* extracting shared code into a package is the right call at all (versus a shared internal namespace within a monorepo, for instance) — settle that decision first if it's not already made.
- `release` handles tagging/publishing/changelog generation once the package is ready to ship a version; `package-developer` covers everything up to that point.

## Package Skeleton

A minimal package needs:

```json
{
    "name": "vendor/package-name",
    "description": "Short description.",
    "license": "MIT",
    "require": {
        "php": "^8.2",
        "illuminate/support": "^12.0|^13.0"
    },
    "require-dev": {
        "orchestra/testbench": "^10.0|^11.0",
        "pestphp/pest": "^3.0"
    },
    "autoload": {
        "psr-4": { "Vendor\\PackageName\\": "src/" }
    },
    "autoload-dev": {
        "psr-4": { "Vendor\\PackageName\\Tests\\": "tests/" }
    },
    "extra": {
        "laravel": {
            "providers": ["Vendor\\PackageName\\PackageNameServiceProvider"],
            "aliases": { "PackageName": "Vendor\\PackageName\\Facades\\PackageName" }
        }
    }
}
```

- `name` must be `vendor/package` form (lowercase, hyphenated) — this is the Packagist/Composer identity, distinct from the PHP namespace.
- The `extra.laravel.providers`/`extra.laravel.aliases` keys drive Laravel's package auto-discovery: Composer writes them into `bootstrap/cache/packages.php` on install, so a consuming app never has to manually register the provider in `bootstrap/providers.php`. Only list a facade in `aliases` if the package actually ships one.
- Keep `require` minimal (`illuminate/support` or the specific `illuminate/*` contracts needed) rather than depending on the full `laravel/framework` — packages should degrade to the narrowest Laravel surface they actually touch.

`spatie/laravel-package-tools` is the de-facto community standard for scaffolding this structure (a `Package` configuration object plus a `PackageServiceProvider` base class that reduces the boilerplate below to a fluent `configurePackage()` call); reach for it when a package needs several of config/migrations/views/commands/assets, and hand-write a plain `ServiceProvider` for something small enough that the abstraction isn't worth the dependency.

## Service Provider Structure

```php
// src/PackageNameServiceProvider.php
declare(strict_types=1);

namespace Vendor\PackageName;

use Illuminate\Support\ServiceProvider;

class PackageNameServiceProvider extends ServiceProvider
{
    public function register(): void
    {
        $this->mergeConfigFrom(__DIR__.'/../config/package-name.php', 'package-name');

        $this->app->singleton(PackageManager::class);
    }

    public function boot(): void
    {
        if ($this->app->runningInConsole()) {
            $this->publishes([
                __DIR__.'/../config/package-name.php' => config_path('package-name.php'),
            ], 'package-name-config');

            $this->publishesMigrations([
                __DIR__.'/../database/migrations' => database_path('migrations'),
            ], 'package-name-migrations');
        }

        $this->loadMigrationsFrom(__DIR__.'/../database/migrations');
        $this->loadViewsFrom(__DIR__.'/../resources/views', 'package-name');
        $this->loadRoutesFrom(__DIR__.'/../routes/web.php');
    }
}
```

- `register()` is for container bindings and `mergeConfigFrom()` only — do not resolve other services here, since not every provider is guaranteed to have registered yet.
- `boot()` is where `publishes()` (config/views/assets) and `publishesMigrations()` (Laravel 11+; auto-updates the migration's timestamp to "now" on publish, replacing the older pattern of pairing plain `publishes()` with `loadMigrationsFrom()` for the publish side) belong, alongside `loadMigrationsFrom()`, `loadViewsFrom()`, and `loadRoutesFrom()` so the package works even if the consumer never publishes anything.
- Guard publishing calls with `runningInConsole()` — there's no reason to register publishable paths on every HTTP request.

## Config Publishing

Ship a config file with sensible defaults so the package works with zero setup, while making every option overridable:

```php
// config/package-name.php
return [
    'driver' => env('PACKAGE_NAME_DRIVER', 'default'),
    'timeout' => 30,
];
```

`mergeConfigFrom()` (in `register()`) ensures the package's defaults apply even if the consumer never runs `vendor:publish`; `publishes([...], 'package-name-config')` (in `boot()`) lets them selectively export just the config file via `php artisan vendor:publish --tag=package-name-config` when they need to override values. Use one tag per publishable group (config, migrations, views) rather than one tag for everything, so consumers can publish only what they need.

## Testing With Orchestra Testbench

`orchestra/testbench` (`require-dev`) provides a minimal, Laravel-aware application shell so package tests can exercise real Eloquent models, migrations, and routes without a full host application:

```php
// tests/TestCase.php
declare(strict_types=1);

namespace Vendor\PackageName\Tests;

use Orchestra\Testbench\TestCase as BaseTestCase;
use Vendor\PackageName\PackageNameServiceProvider;

class TestCase extends BaseTestCase
{
    protected function getPackageProviders($app): array
    {
        return [PackageNameServiceProvider::class];
    }

    protected function defineEnvironment($app): void
    {
        $app['config']->set('database.default', 'testing');
    }
}
```

```php
// tests/Feature/ExampleTest.php
use Vendor\PackageName\Tests\TestCase;

uses(TestCase::class);

it('registers the package config', function () {
    expect(config('package-name.driver'))->toBe('default');
});
```

Run migrations from the package itself in tests (`$this->loadMigrationsFrom()` is already wired via the provider under test) rather than requiring a consuming app's schema. Match the Testbench major to the Laravel majors declared in `composer.json`'s `require-dev` constraint — Testbench versions Laravel majors 1:1 (Testbench 10.x <-> Laravel 12, Testbench 11.x <-> Laravel 13), so testing "Laravel 12 and 13 support" means running the suite against both Testbench majors (e.g. a CI matrix), not just the one installed locally.

## Versioning And Compatibility

- Follow semantic versioning for the package's own tags (`v1.2.3`): breaking changes to the package's public API bump the major, even if no Laravel version changed.
- Declare an explicit, bounded Laravel compatibility range rather than leaving it open-ended:

```json
"require": {
    "laravel/framework": "^11.0|^12.0|^13.0"
}
```

  An unbounded constraint (or one that silently includes an unsupported future major) risks a consumer's `composer update` pulling in a Laravel version the package was never tested against; a range capped too low blocks consumers from upgrading their app at all. Widen the range deliberately (test against the new major first), don't just delete the upper bound.
- Drop support for EOL Laravel/PHP majors explicitly in a changelog entry and a major version bump of the package, rather than leaving dead compatibility code in place indefinitely.

## Verification

Possible checks:

```bash
composer validate --strict
vendor/bin/pest              # or vendor/bin/phpunit, run via Testbench
vendor/bin/pint --test
vendor/bin/phpstan analyse
composer test                # if a spatie/laravel-package-tools-style skeleton defines this script
```

Use only commands present in the package's own `composer.json` (a package repo has its own toolchain, separate from any consuming application).

## Final Output

Return what changed (provider/config/migration/view files, publish tags, test setup), the declared Laravel/PHP compatibility range, tests run, Context Summary, and next step.
