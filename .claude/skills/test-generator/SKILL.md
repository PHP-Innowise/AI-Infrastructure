---
name: test-generator
description: Generate PHPUnit or Pest tests for native PHP applications. Use for unit tests, integration tests, HTTP handler tests, input-validation and authorization tests, data-access tests, and coverage gaps.
phase: execution
flow-next: documentation-generator
flow-alternatives: [debugger, coder]
related: [coder, code-reviewer, verify]
---

# Test Generator

## Overview

Create focused tests that prove behavior. Match the project's existing style: PHPUnit classes if the project uses PHPUnit, Pest functions if it uses Pest.

## Test Selection

```
What are you testing?
        |
        |-- Pure PHP logic (services, value objects)?
        |     |-- Unit test with mocked collaborators
        |
        |-- HTTP handler: routing, validation, auth, response?
        |     |-- Integration test against the handler/PSR-7 request
        |
        |-- Persistence (repository/gateway)?
        |     |-- Integration test against a disposable test database
        |
        |-- External service / queue / mail?
        |     |-- Contract test with a test double / fake
```

## Native PHP Test Tools

- PHPUnit `TestCase` or Pest `test()`/`it()`.
- Test doubles: `createMock()`, `createStub()`, or hand-written fakes for interfaces.
- Database isolation: a dedicated SQLite/MySQL test database, wrapped per test in a transaction that is rolled back in `tearDown()`.
- Fixtures/builders/factories to construct entities and rows deterministically.
- PSR-7 request factories to exercise HTTP handlers without booting a server.
- `vfsStream` or a temp directory for filesystem behavior.

## PHPUnit Integration Test Example

```php
<?php

declare(strict_types=1);

namespace Tests\Integration;

use App\Http\Controller\InvitationController;
use PHPUnit\Framework\TestCase;

final class CreateInvitationTest extends TestCase
{
    private \PDO $pdo;

    protected function setUp(): void
    {
        $this->pdo = TestDatabase::migrated();
        $this->pdo->beginTransaction();
    }

    protected function tearDown(): void
    {
        $this->pdo->rollBack();
    }

    public function test_trainer_can_create_invitation(): void
    {
        $controller = new InvitationController(new CreateInvitation($this->pdo));

        $request = RequestFactory::post('/api/invitations', [
            'email' => 'player@example.com',
            'role' => 'player',
        ])->withAttribute('user', TrainerFixture::create($this->pdo));

        $response = $controller->store($request);

        self::assertSame(201, $response->getStatusCode());

        $statement = $this->pdo->query("SELECT COUNT(*) FROM invitations WHERE email = 'player@example.com'");
        self::assertSame(1, (int) $statement->fetchColumn());
    }

    public function test_email_is_required(): void
    {
        $controller = new InvitationController(new CreateInvitation($this->pdo));

        $request = RequestFactory::post('/api/invitations', ['role' => 'player']);

        $this->expectException(ValidationException::class);

        $controller->store($request);
    }
}
```

## Pest Example

```php
<?php

declare(strict_types=1);

it('creates an invitation for a valid request', function (): void {
    $invitation = (new CreateInvitation(TestDatabase::migrated()))->handle(
        new StoreInvitationRequest('player@example.com', 'player', null)
    );

    expect($invitation->email)->toBe('player@example.com');
});
```

## Coverage Priorities

For new backend behavior, cover:

- Happy path.
- Validation failure.
- Authorization failure.
- Missing resource or invalid state.
- Persistence side effects.
- Queued/event/mail side effects if present (via fakes).

## Test Quality Best Practices

- **Structure with AAA:** Arrange, Act, Assert. One logical behavior per test; a clear failure message.
- **Name for behavior:** `test_rejects_invitation_after_expiry`, not `test_invitation2`. The name should read as a spec.
- **Test behavior, not implementation:** assert on outcomes and observable state, not private internals; this keeps tests green through refactors.
- **Pick the smallest real double:**
  - *Stub* — returns canned data (a query result).
  - *Mock* — asserts an interaction happened (an email was sent). Use sparingly.
  - *Fake* — a lightweight working implementation (in-memory repository). Often the cleanest.
  - Avoid over-mocking: mocking everything tests your mocks, not your code. Prefer real objects and fakes at boundaries.
- **Data providers** for the same logic across many inputs (`@dataProvider` / Pest `->with([...])`), instead of copy-pasted tests.
- **Determinism:** inject a `Clock` for time, seed randomness, and never depend on real network/filesystem/order. Fakes over the network.
- **Coverage that matters:** target meaningful branch coverage of business logic (a pragmatic ~80% on core code), not 100% everywhere. Every bug fix gets a regression test first.
- **Mutation testing:** if configured, run Infection (`vendor/bin/infection`) to check tests actually catch changes; a high MSI beats a high line-coverage number.
- **Keep them fast:** unit tests in milliseconds; reserve slow DB/integration tests for behavior that needs them.

## Running Tests

Use the project-standard command:

```bash
vendor/bin/phpunit --filter=CreateInvitationTest
vendor/bin/pest --filter=invitation
```

Then run the broader applicable suite (prefer Composer scripts):

```bash
composer test
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
