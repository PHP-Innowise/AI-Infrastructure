---
name: test-generator
description: Generate Pest or PHPUnit tests for Laravel applications. Use for feature tests (routes), Form Request validation tests, Policy authorization tests, Eloquent model/factory tests, and coverage gaps.
phase: execution
flow-next: documentation-generator
flow-alternatives: [debugger, coder]
related: [coder, code-reviewer, verify]
---

# Test Generator

## Overview

Create focused tests that prove behavior. Match the project's existing style: Pest functions for new test files (preferred), or PHPUnit classes if the project already standardizes on PHPUnit. Both ship with Laravel out of the box.

## Test Selection

```
What are you testing?
        |
        |-- Pure PHP logic (Actions, services, value objects)?
        |     |-- Unit test with mocked collaborators
        |
        |-- HTTP route: routing, validation, auth, response?
        |     |-- Feature test via `$this->get()/post()/actingAs()`
        |
        |-- Eloquent model / query / relationship?
        |     |-- Feature test with `RefreshDatabase` + model factories
        |
        |-- Form Request validation rules?
        |     |-- Feature test asserting `assertSessionHasErrors()` / `assertValid()`
        |
        |-- Policy/Gate authorization?
        |     |-- Feature test asserting `assertForbidden()` / `assertOk()` per role
        |
        |-- Job, Listener, Notification, Mail, or external HTTP call?
        |     |-- Unit/feature test with the matching fake (`Queue::fake()`, `Http::fake()`, ...)
```

## Laravel Test Tools

- Pest `test()`/`it()` (preferred for new files) or PHPUnit `TestCase` for existing PHPUnit suites.
- `Illuminate\Foundation\Testing\RefreshDatabase` to migrate a fresh in-memory/test database per test; `DatabaseTransactions` when migrations are slow and a persistent test database already exists and you only need transactional rollback.
- Model factories (`User::factory()->create()`, `->for()`, `->has()`, states) to build deterministic data instead of hand-written inserts.
- HTTP testing helpers: `$this->get()`, `->post()`, `->postJson()`, `->actingAs($user)`, `->assertOk()`, `->assertRedirect()`, `->assertJson()`, `->assertJsonValidationErrors()`.
- Facade fakes: `Http::fake()`, `Queue::fake()`, `Mail::fake()`, `Event::fake()`, `Notification::fake()`, `Storage::fake('disk')`, `Bus::fake()`.
- `Illuminate\Support\Carbon`/`Date::setTestNow()` (or `$this->travelTo()`) to freeze time deterministically.

## Pest Feature Test Example

```php
<?php

declare(strict_types=1);

use App\Models\User;
use App\Models\Invitation;
use Illuminate\Foundation\Testing\RefreshDatabase;

uses(RefreshDatabase::class);

it('allows a trainer to create an invitation', function (): void {
    $trainer = User::factory()->trainer()->create();

    $response = $this->actingAs($trainer)->postJson('/api/invitations', [
        'email' => 'player@example.com',
        'role' => 'player',
    ]);

    $response->assertCreated();

    $this->assertDatabaseHas('invitations', [
        'email' => 'player@example.com',
    ]);
});

it('rejects an invitation request without an email', function (): void {
    $trainer = User::factory()->trainer()->create();

    $this->actingAs($trainer)
        ->postJson('/api/invitations', ['role' => 'player'])
        ->assertJsonValidationErrors('email');
});

it('forbids a player from creating an invitation', function (): void {
    $player = User::factory()->create();

    $this->actingAs($player)
        ->postJson('/api/invitations', ['email' => 'other@example.com', 'role' => 'player'])
        ->assertForbidden();
});
```

## PHPUnit Feature Test Example

```php
<?php

declare(strict_types=1);

namespace Tests\Feature;

use App\Models\Invitation;
use App\Models\User;
use Illuminate\Foundation\Testing\RefreshDatabase;
use Tests\TestCase;

final class CreateInvitationTest extends TestCase
{
    use RefreshDatabase;

    public function test_trainer_can_create_invitation(): void
    {
        $trainer = User::factory()->trainer()->create();

        $response = $this->actingAs($trainer)->postJson('/api/invitations', [
            'email' => 'player@example.com',
            'role' => 'player',
        ]);

        $response->assertCreated();
        $this->assertDatabaseHas('invitations', ['email' => 'player@example.com']);
    }

    public function test_email_is_required(): void
    {
        $trainer = User::factory()->trainer()->create();

        $this->actingAs($trainer)
            ->postJson('/api/invitations', ['role' => 'player'])
            ->assertJsonValidationErrors('email');
    }
}
```

## Testing Jobs, Listeners, And Fakes

```php
it('dispatches a welcome email job after registration', function (): void {
    Queue::fake();

    $this->postJson('/api/register', [
        'email' => 'new@example.com',
        'password' => 'password',
    ])->assertCreated();

    Queue::assertPushed(SendWelcomeEmail::class);
});

it('sends a notification when an invitation is accepted', function (): void {
    Notification::fake();

    $invitation = Invitation::factory()->create();
    (new AcceptInvitation())->handle($invitation);

    Notification::assertSentTo($invitation->trainer, InvitationAccepted::class);
});
```

Assert on the job/notification/event class and its payload, not on internal implementation details of the fake.

## Coverage Priorities

For new backend behavior, cover:

- Happy path.
- Form Request validation failure.
- Policy/Gate authorization failure.
- Missing resource (404) or invalid state.
- Eloquent persistence side effects (`assertDatabaseHas`/`assertDatabaseMissing`).
- Queued jobs, events, mail, and notifications dispatched (via fakes).

## Test Quality Best Practices

- **Structure with AAA:** Arrange, Act, Assert. One logical behavior per test; a clear failure message.
- **Name for behavior:** `it('rejects invitation after expiry')` / `test_rejects_invitation_after_expiry`, not `test_invitation2`. The name should read as a spec.
- **Test behavior, not implementation:** assert on HTTP responses, database state, and dispatched fakes, not private internals; this keeps tests green through refactors.
- **Pick the smallest real double:**
  - *Stub* — returns canned data (a factory-built model).
  - *Mock* — asserts an interaction happened (`Mail::assertSent(...)`). Use sparingly.
  - *Fake* — Laravel's built-in facade fakes (`Queue::fake()`, `Http::fake()`) are lightweight working implementations. Often the cleanest choice.
  - Avoid over-mocking: mocking everything tests your mocks, not your code. Prefer real Eloquent models and built-in fakes at boundaries.
- **Data providers** for the same logic across many inputs (Pest `->with([...])` or PHPUnit `@dataProvider`), instead of copy-pasted tests.
- **Determinism:** freeze time with `Date::setTestNow()`/`$this->travelTo()`, seed factory randomness (`fake()->seed()`), and fake network/queue/mail — never depend on real external calls or execution order.
- **Coverage that matters:** target meaningful branch coverage of business logic (a pragmatic ~80% on core code), not 100% everywhere. Every bug fix gets a regression test first.
- **Mutation testing:** if configured, run Infection (`vendor/bin/infection`, Pest has a `--mutate` profile) to check tests actually catch changes; a high MSI beats a high line-coverage number.
- **Keep them fast:** prefer `RefreshDatabase` with an in-memory SQLite connection for unit-level feature tests; reserve slower MySQL/Postgres-backed suites for behavior that depends on database-specific features.

## Running Tests

Use the project-standard command:

```bash
php artisan test --filter=CreateInvitationTest
vendor/bin/pest --filter=invitation
vendor/bin/phpunit --filter=CreateInvitationTest
```

Then run the broader applicable suite (prefer Composer/Artisan scripts):

```bash
composer test
php artisan test
```

For larger suites, prefer Laravel's first-party parallel runner to cut CI wall-clock time:

```bash
composer require brianium/paratest --dev
php artisan test --parallel
```

`--parallel` shells out to `brianium/paratest`, splitting the suite across multiple processes (one per CPU core by default; `--processes=N` to override). It is only safe when tests avoid shared mutable state: `RefreshDatabase`/factories per test (as recommended above) keep each test's data isolated, but hardcoded IDs, fixed emails/usernames, or file paths shared across tests can still collide once tests run concurrently across processes.

## Browser/E2E Testing

Pest 4 ships built-in Playwright-powered browser testing (`pestphp/pest-plugin-browser`): `visit()` a page, click, fill forms, and assert on the rendered result, with full access to Laravel's testing API (`RefreshDatabase`, `Event::fake()`, `actingAs()`) inside the same test. Laravel's own docs now recommend it over Laravel Dusk for new projects.

```php
it('lets a user sign in', function (): void {
    $user = User::factory()->create();

    visit('/login')
        ->fill('email', $user->email)
        ->fill('password', 'password')
        ->click('Log in')
        ->assertSee('Dashboard');
});
```

This is distinct from the `browser-verify` skill: Pest browser tests are automated, repeatable regression tests, written once and run in CI forever alongside the rest of the suite. `browser-verify` is a manual/agent-driven, one-off exploratory check of a specific change in a running app, and is not committed to the test suite. Add coverage here for lasting regression protection; use `/browser-verify` to sanity-check a change during development.

## Failure Loop

1. Read the full failure.
2. Fix the root cause.
3. Re-run the focused failing test.
4. Stop after three failed fix attempts and escalate to `/debugger`.

## Final Output

Return:

- Tests created or updated.
- Behavior covered.
- Commands run and results.
- Remaining gaps.
- Context Summary and Next Steps.
