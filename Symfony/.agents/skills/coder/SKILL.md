---
name: coder
description: Implement Symfony backend code using Controller -> Service -> Repository, SOLID, validation, authorization, tests, and Doctrine conventions. For pure behavior-preserving cleanup use refactorer; for structural scaffolding use architecture-implementer.
phase: execution
flow-next: code-reviewer
flow-alternatives: [test-generator, verify]
related: [architect, architecture-implementer, api-designer, database-designer, test-generator, refactorer]
---

# Symfony Coder

## Overview

Implement behavior-changing Symfony backend work using the project's existing conventions and the layered architecture defined by this accelerator.

Read relevant routes/controllers, services, repositories, entities, migrations, Forms/DTOs, voters/security config, Messenger handlers, tests, and specs before editing.

## Scope Boundary

`coder` owns new features, bug fixes, and incidental cleanup needed for that work.

- Use `refactorer` for pure behavior-preserving cleanup.
- Use `architecture-implementer` to scaffold approved structure before feature logic exists.
- Use `database-designer` when Doctrine entity/schema/index/constraint decisions are not obvious.
- Use `coder-frontend` for Twig, Symfony UX, Stimulus, CSS, and progressive frontend behavior.

## Layer Rules

- Controller delegates to one service/use-case method.
- Service owns business decisions, transaction boundaries, and side-effect orchestration.
- Repository owns Doctrine queries and persistence helpers.
- DTOs carry input/output when arrays would be ambiguous.
- Forms or request DTOs validate HTTP input before service calls.
- Entities protect local invariants but do not know HTTP, sessions, controllers, templates, mailers, or queues.
- Domain/service exceptions must be meaningful enough for controllers or exception listeners to map them to stable HTTP errors.
- Add interfaces only when there are multiple implementations, external boundaries, package boundaries, or tests benefit from a narrow contract.
- Constructor injection is the default. Do not fetch services from the container in application code.

## Implementation Quality Bar

Before editing, identify:

- Existing route/controller/command/handler entry point.
- Service method that should own the use case.
- Repository methods needed for reads/writes.
- Validation boundary.
- Authorization boundary.
- Transaction boundary.
- Test layer that should prove the change.

During implementation:

- Keep method parameters typed and explicit.
- Prefer guard clauses and early returns over deep nesting.
- Keep functions small and single-purpose.
- Avoid passing raw arrays past the controller layer unless the project already standardizes on it.
- Prefer immutable request/result DTOs for cross-layer data.
- Keep Doctrine-specific query decisions in repositories.
- Keep response formatting in controllers, presenters, normalizers, serializers, or response DTOs.
- Use `EntityManagerInterface::wrapInTransaction()` or the project-standard transaction pattern for multi-write operations.
- Preserve backward compatibility of public service/repository/controller contracts when possible.
- Never read or edit `.env`; use Symfony config, parameters, env processors, or secrets.

## Good Implementation

```php
final class CreateInvitationController extends AbstractController
{
    #[Route('/api/invitations', methods: ['POST'])]
    public function __invoke(
        #[MapRequestPayload] CreateInvitationInput $input,
        CreateInvitation $createInvitation,
    ): JsonResponse {
        $this->denyAccessUnlessGranted('INVITATION_CREATE');

        $result = $createInvitation($input);

        return $this->json(InvitationResponse::fromResult($result), 201);
    }
}
```

```php
interface InvitationRepository
{
    public function existsForEmail(EmailAddress $email): bool;

    public function add(Invitation $invitation): void;
}

interface TransactionManager
{
    /** @template T @param Closure(): T $operation @return T */
    public function run(Closure $operation): mixed;
}

interface InvitationOutbox
{
    public function append(InvitationCreated $event): void;
}

final readonly class CreateInvitation
{
    public function __construct(
        private InvitationRepository $invitations,
        private TransactionManager $transactions,
        private InvitationOutbox $outbox,
    ) {
    }

    public function __invoke(CreateInvitationInput $input): InvitationResult
    {
        return $this->transactions->run(function () use ($input): InvitationResult {
            $email = EmailAddress::fromString($input->email);

            // The named database unique constraint remains authoritative under concurrency.
            if ($this->invitations->existsForEmail($email)) {
                throw InvitationAlreadyExists::forEmail($email);
            }

            $invitation = Invitation::create($email, InvitationRole::from($input->role));
            $this->invitations->add($invitation);
            $this->outbox->append(new InvitationCreated($invitation->id()));

            return InvitationResult::fromInvitation($invitation);
        });
    }
}
```

```php
final class DoctrineInvitationRepository extends ServiceEntityRepository implements InvitationRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Invitation::class);
    }

    public function existsForEmail(EmailAddress $email): bool
    {
        return null !== $this->createQueryBuilder('invitation')
            ->select('1')
            ->andWhere('invitation.email = :email')
            ->setParameter('email', $email->toString())
            ->setMaxResults(1)
            ->getQuery()
            ->getOneOrNullResult();
    }

    public function add(Invitation $invitation): void
    {
        $this->getEntityManager()->persist($invitation);
    }
}
```

`TransactionManager` and `InvitationOutbox` are real infrastructure boundaries, and the repository contract is the application service's narrow persistence port. A Doctrine transaction adapter must flush before commit and translate the named unique-constraint violation to `InvitationAlreadyExists`; the outbox adapter must persist its record in that same transaction. Do not add these interfaces to a trivial CRUD workflow when the consuming project does not need substitution or isolation.

## Bad Implementation

```php
final class CreateInvitationController extends AbstractController
{
    public function __invoke(Request $request, EntityManagerInterface $em, MailerInterface $mailer): JsonResponse
    {
        $email = $request->request->get('email');
        $existing = $em->getRepository(Invitation::class)->findOneBy(['email' => $email]);

        if ($existing) {
            return $this->json(['error' => 'exists'], 409);
        }

        $invitation = new Invitation($email);
        $em->persist($invitation);
        $em->flush();
        $mailer->send(...);

        return $this->json($invitation);
    }
}
```

Problems:

- No boundary validation.
- Controller owns query, business rule, persistence, and side effect.
- Raw entity is exposed as JSON.
- Workflow cannot be reused from CLI or Messenger.
- Hard to test without full framework setup.

## SOLID Checklist

- Single Responsibility: each class has one reason to change.
- Open/Closed: new voters, integrations, or strategies can be added without rewriting controllers.
- Liskov: implementations honor repository/service contracts when interfaces exist.
- Interface Segregation: avoid giant interfaces such as `AppRepositoryInterface`.
- Dependency Inversion: services depend on narrow collaborators, not globals or framework helpers.

Review implemented boundaries against [Symfony clean-code patterns](../../../examples/symfony-clean-code-patterns.md). The examples demonstrate responsibility placement and failure modes; adapt types and framework APIs to the consuming project.

## Verification

Run focused tests first, then broader configured checks:

```bash
vendor/bin/phpunit --filter CreateInvitation
composer validate --strict
vendor/bin/phpunit
vendor/bin/phpstan analyse
php bin/console lint:container
```

Report missing tools as `N/A - tooling not configured`.

## Final Output

Include:

- Files changed.
- Layering decisions.
- Tests/checks run.
- Context Summary.
- Next Steps.
