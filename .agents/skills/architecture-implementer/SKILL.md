---
name: architecture-implementer
description: Scaffold and wire an approved architecture into Laravel. Use to turn an architect decision or spec into models, controllers, policies, Form Requests, API Resources, migrations, and Service Provider bindings, ready for feature code. Bridges /architect and /coder.
phase: execution
flow-next: coder
flow-alternatives: [test-generator, code-reviewer, verify]
related: [architect, coder, api-designer, test-generator]
---

# Architecture Implementer

## Overview

Take an approved architecture (from `/architect`, a spec in `specs/`, or a `/council` decision) and lay down the structural skeleton in Laravel: models, controllers, policies, Form Requests, API Resources, migrations, and Service Provider bindings, generated via `artisan make:*` where possible. Leave feature logic thin and clearly marked as TODO for `/coder`.

This skill builds the frame, not the whole house. It should produce a scaffolded, autoloadable, testable skeleton with seams in the right places, using Laravel's own generators to keep boilerplate idiomatic.

## Scope Boundary

This skill stops at the **skeleton**: models, empty/thin controller methods, Form Request shells, policy method stubs, Resource classes, migrations, and Service Provider bindings with TODO markers. The moment real feature logic goes inside a method (validation rules, policy conditions, Action bodies), that is `/coder`. Upstream, the architectural *decision* itself belongs to `/architect` (or `/council`) — if no approved decision exists, do that first. In short: `/architect` decides → `architecture-implementer` scaffolds → `/coder` fills in behavior.

## Preconditions

Before scaffolding, confirm:

- An architecture decision exists (read `specs/architect-architecture.md` or the provided decision). If not, recommend `/architect` first.
- The target models, relationships, authorization needs, and API surface are decided.
- The project's Laravel version (`composer.json` `laravel/framework` constraint or `php artisan --version`) and any conventions already in place (e.g. Actions vs Services directory).

## What To Scaffold

1. **Migrations.** `php artisan make:migration create_invoices_table` (or `add_x_to_y_table`) with columns, foreign keys, and indexes matching the decided schema.
2. **Eloquent models.** `php artisan make:model Invoice -mf` (with migration + factory) or just the model if the migration already exists; declare `$fillable`/`casts()` and relationship method signatures.
3. **Policies.** `php artisan make:policy InvoicePolicy --model=Invoice` with method stubs (`view`, `create`, `update`, `delete`) returning a clear TODO/placeholder.
4. **Form Requests.** `php artisan make:request StoreInvoiceRequest` with an `authorize()` stub and an empty/TODO `rules()` array.
5. **API Resources.** `php artisan make:resource InvoiceResource` (and `InvoiceCollection` if a custom collection shape is needed) with a `toArray()` stub listing the fields to expose.
6. **Controllers.** `php artisan make:controller Api/InvoiceController --api` (or `--resource` for web) with method signatures wired to the Form Request/Resource, bodies marked TODO.
7. **Actions/Services (if the architecture calls for one).** Create the class with a `handle()`/`__invoke()` signature and constructor-injected collaborators, body left as `TODO`/`throw new \RuntimeException('Not implemented')`.
8. **Service Provider bindings.** Register interface-to-implementation bindings in `AppServiceProvider` (or a dedicated provider) for any swappable external dependency the architecture named.
9. **Routes.** Add the route(s) in `routes/api.php`/`routes/web.php` (e.g. `Route::apiResource(...)`) pointing at the new controller, without implementing behavior.
10. **Factories/seeders.** Scaffold a factory (`make:model -f` or `make:factory`) so tests can build fixtures immediately.

## Rules

- Follow Laravel's dependency direction: routes -> controllers -> Actions/Services -> Eloquent models; Service Providers bind interfaces for external integrations. Controllers should not contain business logic.
- Every new PHP file starts with `declare(strict_types=1);` and full type declarations.
- Prefer `artisan make:*` generators over hand-writing boilerplate — they produce the idiomatic base class/namespace/imports automatically.
- Do not implement business rules, validation rules, or policy conditions here; mark them clearly for `/coder`.
- Keep the skeleton verifiable: migrations should run (`php artisan migrate`), models/routes should resolve, and `php artisan route:list` should show the new routes.
- Do not add layers the architecture did not call for (e.g. don't add a Service if only an Action was decided).

## Example Skeleton Shape

```php
<?php

declare(strict_types=1);

namespace App\Models;

use Illuminate\Database\Eloquent\Factories\HasFactory;
use Illuminate\Database\Eloquent\Model;
use Illuminate\Database\Eloquent\Relations\BelongsTo;

final class Invoice extends Model
{
    use HasFactory;

    /** @var list<string> */
    protected $fillable = ['customer_id', 'total_cents', 'status'];

    /** @return BelongsTo<Customer, $this> */
    public function customer(): BelongsTo
    {
        return $this->belongsTo(Customer::class);
    }
}
```

```php
<?php

declare(strict_types=1);

namespace App\Policies;

use App\Models\Invoice;
use App\Models\User;

final class InvoicePolicy
{
    public function view(User $user, Invoice $invoice): bool
    {
        // TODO(coder): implement ownership/role check.
        throw new \RuntimeException('Not implemented');
    }
}
```

```php
<?php

declare(strict_types=1);

namespace App\Actions;

use App\Models\Invoice;

final class IssueInvoice
{
    public function handle(array $data): Invoice
    {
        // TODO(coder): implement issuance rules + DB::transaction().
        throw new \RuntimeException('Not implemented');
    }
}
```

```php
// app/Providers/AppServiceProvider.php
public function register(): void
{
    // TODO(coder): confirm concrete implementation once billing decision is finalized.
    $this->app->bind(PaymentGateway::class, StripePaymentGateway::class);
}
```

## Verification

```bash
composer dump-autoload
php artisan route:list --path=invoices
php artisan migrate --pretend   # confirm migration is syntactically valid without applying
vendor/bin/phpstan analyse      # if configured; confirms wiring types line up
```

## Handoff Map

Produce a table so `/coder` knows exactly what to fill in:

```markdown
| File | Responsibility | Status |
| --- | --- | --- |
| app/Actions/IssueInvoice.php | issuance workflow | TODO: implement |
| app/Policies/InvoicePolicy.php | authorization rules | TODO: implement checks |
| app/Http/Requests/StoreInvoiceRequest.php | validation rules | TODO: implement rules() |
```

## Final Output

Return the created structure, models/policies/resources/migrations added, Service Provider bindings, the handoff map of TODOs, verification run, Context Summary, and next step (`/coder` to implement, then `/test-generator`).
