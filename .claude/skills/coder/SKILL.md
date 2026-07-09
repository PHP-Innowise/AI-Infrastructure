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
- Never read or edit `.env`; read configuration through a config layer and document required variables.

## Common Patterns

### Front Controller

```php
<?php

declare(strict_types=1);

use App\Http\Kernel;

require __DIR__ . '/../vendor/autoload.php';

$kernel = require __DIR__ . '/../config/bootstrap.php';

$kernel->handle(
    ServerRequestFactory::fromGlobals()
)->send();
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
    public function __construct(private readonly CreateUser $createUser)
    {
    }

    public function store(ServerRequestInterface $request): ResponseInterface
    {
        $data = CreateUserRequest::fromArray((array) $request->getParsedBody());

        $user = $this->createUser->handle($data);

        return JsonResponse::created(['id' => $user->id, 'email' => $user->email]);
    }
}
```

### Use Case / Service With Transaction

```php
<?php

declare(strict_types=1);

namespace App\Application;

use App\Http\Request\CreateUserRequest;
use App\Domain\User;
use PDO;

final class CreateUser
{
    public function __construct(private readonly PDO $pdo)
    {
    }

    public function handle(CreateUserRequest $request): User
    {
        $this->pdo->beginTransaction();

        try {
            $statement = $this->pdo->prepare(
                'INSERT INTO users (email, name) VALUES (:email, :name)'
            );
            $statement->execute([
                'email' => $request->email,
                'name' => $request->name,
            ]);

            $user = new User((int) $this->pdo->lastInsertId(), $request->email, $request->name);

            $this->pdo->commit();

            return $user;
        } catch (\Throwable $e) {
            $this->pdo->rollBack();

            throw $e;
        }
    }
}
```

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
