---
name: test-generator
description: "Generate Symfony tests: controller functional tests, service unit tests, repository integration tests, validation tests, authorization/voter tests, console tests, and Messenger tests."
phase: quality
flow-next: verify
flow-alternatives: [coder, code-reviewer]
---

# Symfony Test Generator

## Goal

Add focused tests that prove behavior without overfitting implementation details.

## Required Context

Inspect `composer.json`, PHPUnit/Pest configuration, bootstrap/kernel setup, existing test base classes, fixtures/factories, database reset strategy, HTTP helpers, Messenger transports, and adjacent tests. Follow the consuming project's conventions before introducing helpers or dependencies.

## Select The Lowest Useful Layer

| Behavior | Preferred test |
| --- | --- |
| Service decisions and orchestration | Unit test with narrow collaborators |
| Route/input/auth/response contract | `WebTestCase`/`KernelBrowser` functional test |
| Doctrine QueryBuilder/DQL/SQL | Kernel-backed repository integration test |
| Entity/value-object invariant | Plain unit test |
| DTO/Form/Validator constraint | Validator/Form integration test |
| Voter/access rule | Voter unit test plus protected-route functional test |
| Messenger handler | Handler unit test; transport integration test when routing/serialization matters |
| Console command | `CommandTester`/`ApplicationTester` |
| Twig/UX behavior | Controller/component test plus browser test when interaction matters |

Do not boot the kernel for behavior that can be proven with a plain object test. Do not mock Doctrine query behavior that must be verified against the configured database platform.

## Coverage Contract

For each changed behavior cover:

1. the primary successful path;
2. the highest-risk invalid or denied path;
3. relevant boundary values and state transitions;
4. concurrency, retry, serialization, or persistence behavior when the feature depends on it;
5. regression reproduction before fixing a reported bug.

Prioritize externally observable behavior. Avoid assertions against private methods, framework internals, incidental call order, generated IDs, or complete HTML snapshots unless those are the contract.

## Test Data

- Prefer existing Foundry factories, fixtures, object mothers, builders, or project helpers.
- Keep defaults valid, realistic, explicit, and overrideable. Seed randomness and freeze/inject clocks.
- Model required relations, tenant ownership, roles, and entity states deliberately.
- Respect unique/database constraints; never use production data, real credentials, or personal information.
- Keep destructive fixture loading isolated to the test database. Do not invoke purging fixture commands without explicit consent.

## Unit-Test Pattern

```php
final class CreateInvitationTest extends TestCase
{
    public function testItRejectsAnExistingInvitation(): void
    {
        $repository = new InMemoryInvitationRepository([
            Invitation::create(EmailAddress::fromString('person@example.test'), InvitationRole::Member),
        ]);

        $createInvitation = new CreateInvitation(
            $repository,
            new ImmediateTransactionManager(),
            new CollectingInvitationOutbox(),
        );

        $this->expectException(InvitationAlreadyExists::class);
        $createInvitation(new CreateInvitationInput('person@example.test', 'member'));
    }
}
```

Use test doubles only at real boundaries. Prefer fakes for stateful collaborators, stubs for returned data, spies for meaningful side effects, and mocks only when an interaction is itself the contract.

## Functional-Test Pattern

```php
final class CreateInvitationControllerTest extends WebTestCase
{
    public function testDeniedUserCannotCreateInvitation(): void
    {
        $client = static::createClient();
        $client->loginUser(TestUserFactory::member());
        $client->jsonRequest('POST', '/api/invitations', ['email' => 'person@example.test']);

        self::assertResponseStatusCodeSame(403);
    }
}
```

Functional tests should assert route/method, validation status and violation paths, authentication/authorization, redirects or JSON contract, CSRF for web forms, and absence of sensitive fields. Use the project authentication helper rather than disabling security.

## Doctrine And Transaction Tests

- Boot the kernel and use the real repository/entity manager for custom queries.
- Exercise empty, single, multiple, ordering, pagination, relation, uniqueness, and locking behavior relevant to the query.
- Use the configured reset/transaction mechanism; close or clear the entity manager when proving persisted state rather than relying on the identity map.
- Test database constraints when correctness depends on concurrency or uniqueness.
- Keep platform-specific SQL tests explicit and run them on the supported CI database when possible.

## Forms, Validation, Security, Messenger, And Console

- Form/Validator: valid data, each high-risk constraint, nested property paths, transformation failure, validation groups, and CSRF behavior.
- Voter: supported/unsupported attributes and subjects, anonymous/denied/allowed, wrong owner/tenant, privileged role, and relevant object states.
- Messenger: immutable payload, serialization compatibility, handler delegation, duplicate delivery/idempotency, retry classification, terminal failure, and transaction timing.
- Console: arguments/options, invalid input, non-interactive mode, stdout/stderr, service failure, and exit codes.

## Determinism And Quality

- Use Arrange-Act-Assert and descriptive behavior names.
- Use data providers for the same behavior over meaningful cases, not to hide unrelated scenarios.
- Avoid sleeps, network access, shared mutable globals, unordered assertions, and dependence on test execution order.
- Freeze clocks, inject UUID/ID generators, use fake transports/clients, and set locale/timezone where output depends on them.
- Clean up filesystem and external resources even when assertions fail.
- Add mutation testing only when already configured; never claim line coverage proves behavioral quality.

Use the behavior-focused examples in [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). Test public behavior at the layer that owns it and avoid mocks coupled to private implementation details.

## Run And Failure Loop

Run the narrow test first, then the relevant suite and configured quality gates:

```bash
vendor/bin/phpunit --filter CreateInvitation
vendor/bin/phpunit
composer test
```

If a test fails, read the full failure, confirm whether production behavior or the test assumption is wrong, make one evidence-based correction, and rerun the narrow test. Do not weaken an assertion merely to obtain green output.

## Output

Report tests added, behavior and risk paths covered, fixtures/factories introduced, commands and results, unavailable tooling, and intentional remaining gaps. Include Context Summary and Next Steps.
