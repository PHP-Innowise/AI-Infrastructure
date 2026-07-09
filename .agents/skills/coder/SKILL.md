---
name: coder
description: Implement native PHP backend features and bug fixes (behavior-changing work). Use for HTTP handlers/controllers, routing, input validation, domain services, PDO data access, value objects, and PHP business logic. For pure behavior-preserving cleanups use refactorer; for scaffolding an approved architecture use architecture-implementer.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, verify]
related: [architect, architecture-implementer, api-designer, test-generator]
---

# Coder

## Overview

Implement backend work in plain, framework-agnostic PHP using the project's existing conventions. Read the relevant entry points, routing, handlers, services, data access, tests, and specs before editing.

This skill targets native PHP (Composer + PSR). If the project is built on a framework, prefer the matching accelerator branch instead of hand-rolling framework features here.

## Scope Boundary

`coder` owns **behavior-changing** work — new features, bug fixes, and the incidental cleanup that comes with them. Use a sibling skill when the task is narrower:

- **`/refactorer`** — pure behavior-preserving change (extract, dedupe, improve types, PHP upgrade) under a characterization test net. If observable behavior must stay identical, that is refactorer, not coder.
- **`/architecture-implementer`** — lay down the structural skeleton (directories, namespaces, interfaces, DI wiring) for an approved architecture before feature logic exists. If there is no structure yet from `/architect`, scaffold there first, then return here to fill in behavior.
- **`/coder-frontend`** — server-rendered templates, HTML, CSS, and progressive-enhancement JS.

## Project Structure

A conventional native PHP layout:

```
public/
└── index.php          # front controller (single entry point)

src/                   # PSR-4 autoloaded application code
├── Http/
│   ├── Controller/    # or Action/Handler classes
│   ├── Middleware/    # PSR-15 middleware
│   └── Request/       # input DTOs / validators
├── Domain/            # entities, value objects, domain services
├── Application/       # use cases / service layer
└── Infrastructure/
    ├── Persistence/   # PDO gateways / repositories
    └── Support/       # clients, adapters

config/                # config arrays, DI definitions
bin/                   # CLI entry points
templates/             # server-rendered views (if any)
tests/
├── Unit/
└── Integration/
```

Locate the app root by finding `composer.json` and the autoload `psr-4` mapping. Follow the structure already present rather than imposing this one.

## Workflow

1. **Understand** - read existing implementation, specs, schema, and tests.
2. **Plan** - decide entry point, validation, domain/service, persistence, and tests.
3. **Implement** - keep changes scoped, typed, and idiomatic.
4. **Test** - run focused tests first, then applicable DoD checks.
5. **Review** - check input validation, authorization, and data integrity.

## Native PHP Implementation Rules

- Start new files with `declare(strict_types=1);` and full type declarations.
- Keep controllers/handlers thin: parse and validate input, call a service, format a response.
- Validate and normalize input into typed DTOs or value objects at the boundary.
- Authorize protected actions through an explicit access-control check, not hidden UI.
- Access the database through PDO with prepared statements and bound parameters.
- Depend on interfaces at boundaries; inject collaborators via the constructor (PSR-11 container or manual wiring).
- Use versioned migrations or reviewed SQL for schema changes.
- Throw typed exceptions for error conditions; map them to responses/exit codes at the edge.
- Prefer guard clauses and early returns over deep nesting; keep functions small and single-purpose.
- Preserve backward compatibility of public signatures: add optional parameters rather than reordering/removing, and deprecate (with a documented path) before breaking callers.
- Never read or edit `.env`; read configuration through a config layer and document required variables.

## Common Patterns

> Note: helper types below (`JsonResponse`, `ValidationException`, `ServerRequestFactory`, `SapiEmitter`) are illustrative stand-ins for whatever your project/PSR packages provide. Import them with `use` in real code.

### Front Controller

A PSR-7 `ResponseInterface` is a value object with no `send()` method; emit it through an SAPI emitter (for example `laminas/httphandlerrunner`) or a project-specific emitter.

```php
<?php

declare(strict_types=1);

use Laminas\HttpHandlerRunner\Emitter\SapiEmitter;
use Laminas\Diactoros\ServerRequestFactory;

require __DIR__ . '/../vendor/autoload.php';

/** @var App\Http\Kernel $kernel */
$kernel = require __DIR__ . '/../config/bootstrap.php';

$request = ServerRequestFactory::fromGlobals();
$response = $kernel->handle($request);

(new SapiEmitter())->emit($response);
```

### Routing (framework-agnostic, e.g. nikic/fast-route)

```php
$dispatcher = FastRoute\simpleDispatcher(function (FastRoute\RouteCollector $r): void {
    $r->addRoute('GET', '/users/{id:\d+}', [UserController::class, 'show']);
    $r->addRoute('POST', '/users', [UserController::class, 'store']);
});
```

### Input Validation DTO

```php
<?php

declare(strict_types=1);

namespace App\Http\Request;

final class CreateUserRequest
{
    public function __construct(
        public readonly string $email,
        public readonly string $name,
    ) {
    }

    /** @param array<string, mixed> $input */
    public static function fromArray(array $input): self
    {
        $errors = [];

        $email = trim((string) ($input['email'] ?? ''));
        if ($email === '' || filter_var($email, FILTER_VALIDATE_EMAIL) === false) {
            $errors['email'] = 'A valid email is required.';
        }

        $name = trim((string) ($input['name'] ?? ''));
        if ($name === '') {
            $errors['name'] = 'Name is required.';
        }

        if ($errors !== []) {
            throw ValidationException::withErrors($errors);
        }

        return new self($email, $name);
    }
}
```

### Controller / Handler

```php
<?php

declare(strict_types=1);

namespace App\Http\Controller;

use App\Application\CreateUser;
use App\Http\Request\CreateUserRequest;
use Psr\Http\Message\ResponseInterface;
use Psr\Http\Message\ServerRequestInterface;

final class UserController
{
    public function __construct(
        private readonly CreateUser $createUser,
        private readonly AccessControl $accessControl,
    ) {
    }

    public function store(ServerRequestInterface $request): ResponseInterface
    {
        // Authorize before acting; never rely on hidden UI. Use $request attributes
        // populated by auth middleware. (Omit only for genuinely public endpoints.)
        $this->accessControl->assertCan($request->getAttribute('user'), 'user.create');

        $data = CreateUserRequest::fromArray((array) $request->getParsedBody());

        $user = $this->createUser->handle($data);

        return JsonResponse::created(['id' => $user->id, 'email' => $user->email]);
    }
}
```

### Use Case / Service With Transaction

The Application layer depends on an **interface** (defined in the domain) and a small transaction boundary, not on `PDO` directly. SQL lives in the Infrastructure implementation. This keeps the use case unit-testable with a fake repository and honors the "depend on abstractions" rule.

```php
<?php

declare(strict_types=1);

namespace App\Domain\User;

interface UserRepository
{
    /** @throws DuplicateEmailException */
    public function add(User $user): User;
}
```

```php
<?php

declare(strict_types=1);

namespace App\Application;

use App\Domain\User\{User, UserRepository};
use App\Http\Request\CreateUserRequest;
use App\Infrastructure\Persistence\TransactionManager;

final class CreateUser
{
    public function __construct(
        private readonly UserRepository $users,
        private readonly TransactionManager $transaction,
    ) {
    }

    public function handle(CreateUserRequest $request): User
    {
        // Run the write inside a transaction boundary; the manager commits on
        // success and rolls back on any thrown exception.
        return $this->transaction->run(
            fn (): User => $this->users->add(
                User::register($request->email, $request->name),
            ),
        );
    }
}
```

```php
<?php

declare(strict_types=1);

namespace App\Infrastructure\Persistence;

use App\Domain\User\{DuplicateEmailException, User, UserRepository};
use PDO;

final class PdoUserRepository implements UserRepository
{
    public function __construct(private readonly PDO $pdo)
    {
    }

    public function add(User $user): User
    {
        try {
            $statement = $this->pdo->prepare(
                'INSERT INTO users (email, name) VALUES (:email, :name)'
            );
            $statement->execute(['email' => $user->email, 'name' => $user->name]);
        } catch (\PDOException $e) {
            // Rely on a UNIQUE(email) constraint to close the check-then-insert
            // race; translate the driver error into a domain exception.
            if ($e->getCode() === '23000') {
                throw new DuplicateEmailException($user->email, previous: $e);
            }

            throw $e;
        }

        return $user->withId((int) $this->pdo->lastInsertId());
    }
}
```

Enforce uniqueness in the schema (`UNIQUE(email)`), not just in application code — concurrent requests can both pass an application-level "does this email exist?" check. See `/architect` for concurrency, idempotency, and locking guidance the code must respect.

## Modern PHP Best Practices

Use the language's current features to make intent explicit and errors unrepresentable:

- **Enums** for closed sets instead of string/int constants: `enum Role: string { case Trainer = 'trainer'; ... }`.
- **`readonly` properties / classes** for value objects and DTOs so state cannot mutate after construction.
- **Constructor property promotion** to keep DTOs/services concise.
- **`match`** (strict, exhaustive) over long `switch`/`if` ladders.
- **Named arguments** for calls with several optional parameters; **nullsafe** `?->` for optional chains.
- **First-class callable syntax** (`$this->handle(...)`) for cleaner callbacks.
- **`never` return type** for functions that always throw or exit.
- Avoid `mixed` where a union or generic-via-docblock (`@param list<User>`) is clearer; let PHPStan/Psalm enforce it.

## Error Handling

- Define a small typed exception hierarchy (e.g. `DomainException` base, `ValidationException`, `NotFoundException`, `ConflictException`). Do not throw bare `\Exception`.
- Throw at the point of failure; catch only where you can add value (map to a response, add context, retry). Never swallow with an empty `catch`.
- Map exceptions to HTTP status/CLI exit codes at the edge (a single error-handling middleware or handler), not scattered through the code.
- Preserve the original cause with the `$previous` argument when re-throwing.
- Fail fast on programmer errors (invalid state) with exceptions; reserve return-value error signalling for expected, recoverable outcomes.

## Logging (PSR-3)

- Depend on `Psr\Log\LoggerInterface`, not a concrete logger; inject it. Use `NullLogger` in tests.
- Use appropriate levels (`error` for failures needing attention, `warning` for recoverable anomalies, `info` for milestones, `debug` for diagnostics).
- Log structured context as the second argument (`$logger->error('Payment failed', ['order_id' => $id])`); never log secrets, tokens, passwords, or full PII.

## Configuration

- Read config through a typed config layer/array, never `getenv()` scattered across the code or `.env` reads in app logic.
- Validate required config at bootstrap and fail loudly if missing.
- Keep environment-specific values out of source; document required variables by name.

## Middleware (PSR-15)

Use middleware for cross-cutting concerns (auth, CSRF, rate limiting, logging, error handling) so handlers stay focused:

```php
final class AuthenticationMiddleware implements Psr\Http\Server\MiddlewareInterface
{
    public function process(ServerRequestInterface $request, RequestHandlerInterface $handler): ResponseInterface
    {
        $user = $this->authenticator->fromRequest($request);
        if ($user === null) {
            return JsonResponse::unauthorized();
        }

        return $handler->handle($request->withAttribute('user', $user));
    }
}
```

## Verification

Run focused checks first:

```bash
vendor/bin/phpunit --filter=CreateUserTest
```

Then run applicable project checks (prefer Composer scripts):

```bash
composer validate --strict
composer test        # or vendor/bin/phpunit / vendor/bin/pest
composer lint        # or vendor/bin/php-cs-fixer fix --dry-run --diff / vendor/bin/phpcs
composer analyse     # or vendor/bin/phpstan analyse / vendor/bin/psalm
```

If a tool is missing, report `N/A - tooling not configured`.

## Final Output

Include:

- What changed.
- Tests/checks run.
- Any security or migration notes.
- Context Summary.
- Next by flow: `/code-reviewer`, `/test-generator`, or `/verify`.
