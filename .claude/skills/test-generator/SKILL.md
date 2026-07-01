---
name: test-generator
description: Generate PHPUnit or Pest tests for Laravel applications. Use for unit tests, feature tests, API tests, authorization tests, validation tests, queue/event fakes, and coverage gaps.
phase: execution
flow-next: documentation-generator
flow-alternatives: [debugger, coder]
related: [coder, code-reviewer, verify]
---

# Test Generator

## Overview

Create focused Laravel tests that prove behavior. Match the project's existing style: PHPUnit classes if the project uses PHPUnit, Pest functions if it uses Pest.

## Test Selection

```
What are you testing?
        |
        |-- Pure PHP logic?
        |     |-- Unit test
        |
        |-- HTTP request, validation, auth, DB behavior?
        |     |-- Feature test
        |
        |-- Queue, event, mail, notification, external service?
        |     |-- Feature/integration test with Laravel fakes
        |
        |-- Browser workflow?
              |-- Browser/E2E tooling if configured
```

## Laravel Test Tools

- `RefreshDatabase` for database-backed feature tests.
- Factories for model setup.
- `actingAs()` for authenticated requests.
- `Event::fake()`, `Queue::fake()`, `Notification::fake()`, `Mail::fake()`, and `Storage::fake()` for side effects.
- `Http::fake()` for external API calls.
- Policy assertions through HTTP status checks and direct policy tests when rules are complex.

## PHPUnit Feature Test Example

```php
namespace Tests\Feature;

use App\Models\User;
use Illuminate\Foundation\Testing\RefreshDatabase;
use Tests\TestCase;

class CreateInvitationTest extends TestCase
{
    use RefreshDatabase;

    public function test_trainer_can_create_invitation(): void
    {
        $trainer = User::factory()->trainer()->create();

        $response = $this->actingAs($trainer)
            ->postJson('/api/invitations', [
                'email' => 'player@example.com',
                'role' => 'player',
            ]);

        $response->assertCreated()
            ->assertJsonPath('data.email', 'player@example.com');

        $this->assertDatabaseHas('invitations', [
            'email' => 'player@example.com',
            'role' => 'player',
        ]);
    }

    public function test_email_is_required(): void
    {
        $trainer = User::factory()->trainer()->create();

        $this->actingAs($trainer)
            ->postJson('/api/invitations', ['role' => 'player'])
            ->assertUnprocessable()
            ->assertJsonValidationErrors(['email']);
    }
}
```

## Pest Example

```php
use App\Models\User;
use Illuminate\Foundation\Testing\RefreshDatabase;

uses(RefreshDatabase::class);

it('allows a trainer to create an invitation', function (): void {
    $trainer = User::factory()->trainer()->create();

    $this->actingAs($trainer)
        ->postJson('/api/invitations', [
            'email' => 'player@example.com',
            'role' => 'player',
        ])
        ->assertCreated()
        ->assertJsonPath('data.email', 'player@example.com');
});
```

## Coverage Priorities

For new backend behavior, cover:

- Happy path.
- Validation failure.
- Authorization failure.
- Missing resource or invalid state.
- Database side effects.
- Queued/event/mail/notification side effects if present.

## Running Tests

Use the project-standard command:

```bash
php artisan test --filter=CreateInvitationTest
vendor/bin/pest --filter=invitation
vendor/bin/phpunit --filter CreateInvitationTest
```

Then run the broader applicable suite:

```bash
php artisan test
```

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
