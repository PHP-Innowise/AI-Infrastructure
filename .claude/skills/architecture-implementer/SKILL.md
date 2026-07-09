---
name: architecture-implementer
description: Scaffold and wire an approved architecture into native PHP. Use to turn an architect decision or spec into module skeletons, interfaces, DI wiring, PSR-4 structure, and boundaries, ready for feature code. Bridges /architect and /coder.
phase: execution
flow-next: coder
flow-alternatives: [test-generator, code-reviewer, verify]
related: [architect, coder, api-designer, test-generator]
---

# Architecture Implementer

## Overview

Take an approved architecture (from `/architect`, a spec in `specs/`, or a `/council` decision) and lay down the structural skeleton in native PHP: directories, namespaces, interfaces, base classes, DI wiring, and boundaries. Leave feature logic thin and clearly marked as TODO for `/coder`.

This skill builds the frame, not the whole house. It should produce a compiling, autoloadable, testable skeleton with seams in the right places.

## Preconditions

Before scaffolding, confirm:

- An architecture decision exists (read `specs/architect-architecture.md` or the provided decision). If not, recommend `/architect` first.
- The target module boundaries, dependency direction, and persistence approach are decided.
- The project's PHP version and PSR-4 autoload mapping (from `composer.json`).

## What To Scaffold

1. **PSR-4 structure.** Create namespaced directories that match the composer autoload map (e.g. `src/Billing/{Http,Application,Domain,Infrastructure}`).
2. **Interfaces at boundaries.** Define the contracts the inner layers depend on (e.g. `InvoiceRepository`, `PaymentGateway`, `Clock`), owned by Domain/Application, implemented in Infrastructure.
3. **Skeleton classes.** Create handlers/controllers, use-case classes, entities/value objects, and repository implementations with signatures and docblocks, but minimal bodies (throw `RuntimeException('Not implemented')` or return typed placeholders where a stub is safer).
4. **Dependency injection wiring.** Register bindings in the container definition or a manual factory (`config/container.php` / `config/bootstrap.php`), mapping each interface to its implementation.
5. **Entry-point wiring.** Add routes/entry points that resolve the new handlers, without implementing behavior.
6. **Seams for tests.** Ensure collaborators are injected (constructor) so they can be mocked; avoid `new` for cross-boundary dependencies.

## Rules

- Follow dependency direction: entry -> application -> domain; infrastructure implements inner interfaces. No inner layer imports infrastructure.
- Every new file starts with `declare(strict_types=1);` and full type declarations.
- Do not implement business rules here; mark them clearly for `/coder`.
- Keep the skeleton verifiable: it must autoload and pass `php -l`, and ideally a trivial "wiring resolves" test.
- Do not add layers the architecture did not call for.

## Example Skeleton Shape

```php
<?php

declare(strict_types=1);

namespace App\Billing\Domain;

interface InvoiceRepository
{
    public function ofId(int $id): ?Invoice;

    public function save(Invoice $invoice): void;
}
```

```php
<?php

declare(strict_types=1);

namespace App\Billing\Application;

use App\Billing\Domain\InvoiceRepository;

final class IssueInvoice
{
    public function __construct(private readonly InvoiceRepository $invoices)
    {
    }

    public function handle(IssueInvoiceCommand $command): int
    {
        // TODO(coder): implement issuance rules + transaction.
        throw new \RuntimeException('Not implemented');
    }
}
```

```php
// config/container.php
$container->set(
    App\Billing\Domain\InvoiceRepository::class,
    fn ($c) => new App\Billing\Infrastructure\PdoInvoiceRepository($c->get(PDO::class))
);
```

## Verification

```bash
composer dump-autoload
find src -name '*.php' -print0 | xargs -0 -n1 php -l
composer analyse   # if configured; confirms wiring types line up
```

## Handoff Map

Produce a table so `/coder` knows exactly what to fill in:

```markdown
| File | Responsibility | Status |
| --- | --- | --- |
| src/Billing/Application/IssueInvoice.php | issuance workflow | TODO: implement |
| src/Billing/Infrastructure/PdoInvoiceRepository.php | persistence | TODO: implement queries |
```

## Final Output

Return the created structure, interfaces and wiring added, the handoff map of TODOs, verification run, Context Summary, and next step (`/coder` to implement, then `/test-generator`).
