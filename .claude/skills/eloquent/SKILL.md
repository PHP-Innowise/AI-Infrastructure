---
name: eloquent
description: Deep Eloquent ORM patterns - polymorphic relationships, accessors/mutators via Attribute casts, custom cast classes, local/global query scopes, model events and Observers, mass-assignment protection, and advanced eager loading/large-dataset iteration. Use when implementing or reviewing model-layer behavior beyond basic CRUD.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, performance-optimization]
related: [database-designer, coder, performance-optimization]
---

# Eloquent

## Overview

Implement or review model-layer behavior once the underlying schema exists: polymorphic relationships, modern accessors/mutators, custom casts, query scopes, model events/Observers, mass-assignment protection, and eager loading/iteration strategy for large datasets. This is the layer between "the table exists" and "the controller calls a clean model API."

This branch targets Laravel (PHP 8.2+, 8.3+ required for Laravel 13). Supports Laravel 12 (current LTS) and Laravel 13 (current release). For framework-agnostic native PHP, use the `main` branch instead.

## Scope Boundary

`eloquent` owns **model-layer behavior**, not schema or feature wiring. Use a sibling skill when the task is different:

- **Schema/migrations, foreign keys, indexes, and which relationship type maps to which table shape** (`hasOne`/`hasMany`/`belongsTo`/`belongsToMany`) -> `database-designer` first. `eloquent` assumes the tables and foreign keys already exist and focuses on what the model *does* with that schema.
- **A full feature end-to-end** (route, Form Request, controller, Action, tests) where the model change is incidental -> `coder`. Reach for `eloquent` when the model-layer behavior itself — a polymorphic relation, a tricky scope, an Observer, a cast — is the non-trivial part of the task.
- **Query is correct but slow** (N+1 already fixed, still too slow; indexing; caching) -> `performance-optimization`.

## Polymorphic Relationships

Use a polymorphic relation when one model can belong to more than one other model type through a single relationship, backed by a `*_type` / `*_id` column pair (`morphs()` in a migration).

```php
// Comment belongs to either a Post or a Video via commentable_type / commentable_id
final class Comment extends Model
{
    /** @return MorphTo<Model, $this> */
    public function commentable(): MorphTo
    {
        return $this->morphTo();
    }
}

final class Post extends Model
{
    /** @return MorphMany<Comment, $this> */
    public function comments(): MorphMany
    {
        return $this->morphMany(Comment::class, 'commentable');
    }
}
```

`morphToMany` covers many-to-many polymorphism (e.g. a `Tag` attached to both `Post` and `Video` through a `taggables` pivot with `tag_id` / `taggable_type` / `taggable_id`):

```php
final class Tag extends Model
{
    /** @return MorphToMany<Post, $this> */
    public function posts(): MorphToMany
    {
        return $this->morphedByMany(Post::class, 'taggable');
    }
}
```

By default the `*_type` column stores the fully qualified class name (`App\Models\Post`), which leaks internal namespace structure into the database and breaks if a model is ever renamed/moved. Register a `MorphMap` in `AppServiceProvider::boot()` so the column stores a stable alias instead:

```php
use Illuminate\Database\Eloquent\Relations\Relation;

Relation::enforceMorphMap([
    'post' => Post::class,
    'video' => Video::class,
]);
```

Prefer `enforceMorphMap()` over the older `morphMap()` on a new project — it throws `ClassMorphViolationException` for any unmapped model instead of silently falling back to the class name, catching a missing alias immediately. Adding a morph map to an *existing* app with live data requires backfilling every stored `*_type` value from the class name to the alias first.

## Accessors And Mutators

Define accessors/mutators with the modern `Attribute::make()` API (Laravel 9+) rather than the legacy `getXAttribute()`/`setXAttribute()` magic methods. The legacy style still works and you will see it in older code, but prefer `Attribute::make()` for anything new — it is explicit about get/set in one place and is fully typed.

```php
use Illuminate\Database\Eloquent\Casts\Attribute;

final class User extends Model
{
    protected function fullName(): Attribute
    {
        return Attribute::make(
            get: fn (mixed $value, array $attributes): string => "{$attributes['first_name']} {$attributes['last_name']}",
        );
    }

    protected function email(): Attribute
    {
        return Attribute::make(
            set: fn (string $value): string => strtolower(trim($value)),
        );
    }
}
```

## Custom Casts And Enums

For a native backed PHP enum, use the plain enum cast — no custom class needed:

```php
protected function casts(): array
{
    return ['status' => OrderStatus::class]; // enum OrderStatus: string { case Pending = 'pending'; ... }
}
```

Write a custom cast class (`implements CastsAttributes`, generated with `php artisan make:cast`) when the attribute needs logic beyond a 1:1 value mapping — combining multiple columns into one value object, encryption/decryption, or a non-trivial serialization format:

```php
final class Money implements CastsAttributes
{
    public function get(Model $model, string $key, mixed $value, array $attributes): MoneyValue
    {
        return new MoneyValue((int) $value, $attributes['currency'] ?? 'USD');
    }

    public function set(Model $model, string $key, mixed $value, array $attributes): array
    {
        return [$key => $value instanceof MoneyValue ? $value->cents : $value];
    }
}
```

Rule of thumb: an enum cast is enough for a closed set of scalar values; reach for a custom cast class only when the attribute's storage representation and its in-memory representation genuinely differ in shape (a value object, multiple columns, encrypted payload).

## Query Scopes

A **local scope** (`scopeActive`) is opt-in per query — safe, explicit, and the default choice:

```php
final class Order extends Model
{
    /** @param Builder<Order> $query */
    public function scopeActive(Builder $query): void
    {
        $query->where('status', OrderStatus::Active);
    }
}

Order::active()->get();
```

A **global scope** (`static::addGlobalScope()`, typically in `booted()`) applies to *every* query against the model automatically, including ones written by other developers who may not know it exists. Reserve global scopes for invariants that must always hold — multi-tenancy (filtering every query to the current tenant) or a soft-delete-like always-on filter — never for convenience filtering that a local scope would express just as well:

```php
final class Order extends Model
{
    protected static function booted(): void
    {
        static::addGlobalScope(new TenantScope);
    }
}
```

A global scope that silently drops rows is exactly the kind of thing that surprises the next developer debugging "why is this record missing" — document it prominently on the model, and give callers an explicit way to bypass it (`withoutGlobalScope(TenantScope::class)`) for the rare legitimate cross-tenant query (e.g. an admin report).

## Model Events And Observers

Eloquent fires model events (`creating`, `created`, `updating`, `updated`, `saving`, `saved`, `deleting`, `deleted`, and more) that can be hooked either inline (`static::created(fn ($order) => ...)` in `booted()`) or, for anything beyond a one-liner, in a dedicated Observer:

```php
// php artisan make:observer OrderObserver --model=Order
final class OrderObserver
{
    public function created(Order $order): void
    {
        Log::info('Order created', ['order_id' => $order->id]);
    }

    public function deleting(Order $order): void
    {
        if ($order->isLocked()) {
            throw new OrderLockedException($order);
        }
    }
}

// AppServiceProvider::boot()
Order::observe(OrderObserver::class);
```

Observers are appropriate for behavior that is intrinsically tied to *this model's* lifecycle and has no meaningful caller-supplied context (auditing, cache invalidation, denormalized counter maintenance, simple validation-of-invariant). Avoid growing a "fat model event soup" the same way the accelerator avoids fat controllers: if the reaction to an event needs to orchestrate other models, call external services, or run in a transaction with the triggering write, that is workflow logic and belongs in an Action/Service called explicitly from the write path — not chained invisibly off an event. A caller reading `OrderAction::handle()` should be able to see what happens; a caller reading `Order::create()` should not have to go hunting through Observers to find out it also charged a card.

## Mass Assignment Protection

Declare `$fillable` (allow-list) or `$guarded` (deny-list) on every model that accepts array/request input via `create()`/`fill()`/`update()`. Prefer `$fillable` — an explicit allow-list fails safely when a new sensitive column (`is_admin`, `role`) is added later, whereas `$guarded` requires remembering to add the new column to the deny-list.

```php
final class User extends Model
{
    /** @var list<string> */
    protected $fillable = ['name', 'email'];
}
```

`Model::unguard()` or `protected $guarded = []` disables mass-assignment protection entirely — every column becomes writable from an array, including ones no Form Request validates. This is appropriate inside a factory/seeder (trusted, test-only data) but dangerous anywhere near request input: a single `User::create($request->all())` on an unguarded model is a direct privilege-escalation vector (an attacker adding `is_admin=1` to the payload). Never unguard a model that is ever hydrated from user-controlled data.

## Advanced Eager Loading

Constrain an eager load with a closure instead of loading the full relation:

```php
$posts = Post::with(['comments' => fn ($query) => $query->latest()->limit(5)])->get();
```

Nested eager loading nests relation names with dot syntax:

```php
$posts = Post::with('author.profile')->get();
```

Use `withCount()`, `withExists()`, or `withAggregate()` (`withSum()`, `withMax()`, ...) when the caller only needs a count/boolean/aggregate, not the full related rows — this avoids loading and hydrating relation models just to call `->count()` or `->isNotEmpty()` in PHP:

```php
$posts = Post::withCount('comments')->withExists('comments as has_pinned_comment')->get();
// $post->comments_count, $post->has_pinned_comment
```

## Large-Dataset Iteration

Choose based on memory profile and whether the loop mutates rows it is iterating over:

| Method | Queries | Memory | Use when |
| --- | --- | --- | --- |
| `chunk($n)` | Many (offset-based) | Low | Simple batch processing; rows are **not** being updated/deleted in a way that shifts the offset |
| `chunkById($n)` | Many (keyed by id) | Low | Same as `chunk()` but safe when rows are being mutated during iteration (keys by last-seen ID instead of offset) |
| `cursor()` | One | Very low (single unbuffered query, hydrates one model at a time) | Read-only iteration over a large set; do not call further queries that mutate the same table inside the loop |
| `lazy()` / `lazyById()` | Many, wrapped in a `LazyCollection` | Low | Same trade-offs as `chunk()`/`chunkById()` but with a `LazyCollection`'s chainable API (`filter()`, `map()`, ...) |

```php
Order::where('status', OrderStatus::Pending)->chunkById(500, function (Collection $orders): void {
    $orders->each(fn (Order $order) => $order->cancel());
});
```

Pitfall: `chunk()` (and, less obviously, `chunkById()` if the mutation changes the ordered column) can skip or re-process rows if the collection being iterated is itself being updated/deleted in a way that changes which rows match the underlying `WHERE`/`ORDER BY` — for example, updating the same `status` column the chunk query filters on. When rows must be mutated during the loop, either mutate an unrelated column, re-derive the ID cursor explicitly, or collect IDs first and iterate a stable list.

## Verification

Possible checks:

```bash
php artisan test --filter=Order
vendor/bin/pint --test
vendor/bin/phpstan analyse   # Larastan resolves Eloquent relationship generics (HasMany<Order, $this>) when phpdoc return types are present
```

Use only commands present in the project.

## Final Output

Return what changed (models, casts, scopes, Observers, migrations for morph columns if any), the reasoning behind any global scope or custom cast, tests run, Context Summary, and next step.
