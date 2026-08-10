<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Service\AuditLogger;

/**
 * "Adopting a trainer's tenant without becoming a user" — the second of the
 * two mechanisms that let Super Admin write a trainer-scoped row
 * (architect-architecture.md "Writing across tenants"). Unlike
 * impersonation, `getUser()` never changes: the Super Admin's own identity
 * stays in the token throughout, which is exactly why every voter clause
 * that relies on this must be explicit (`ROLE_SUPER_ADMIN` **and**
 * `isOpenFor($subject->getTrainer())`) rather than falling out of an
 * ownership check the way impersonation's identity swap does.
 *
 * Scope lifetime is one request/response cycle only
 * (specs/api-designer-spec.md Decisions, "AdministrativeScope lifetime") —
 * this class holds no session state, only an in-memory flag for the
 * duration of the current request. The two named callers are Epic-02's own
 * Event Master slice (this module) and Epic-05's per-trainer fee edit, not
 * yet built.
 *
 * First built here, in Epic-02, for the Event Master minimal slice
 * (AC-02-55..57) — architect-architecture.md's Module map assigns
 * "administrative tenant scope" to `Platform`, not `Administration`,
 * because it is foundational tenancy machinery Administration's screens
 * call into, the same relationship `CrossTenantReadService` has.
 *
 * @see specs/architect-architecture.md "Writing across tenants", "Cross-cutting services"
 * @see specs/security-voter-designer-design.md "Super Admin: impersonation and administrative tenant scope"
 */
final class AdministrativeScope
{
    private ?int $openForTrainerId = null;

    public function __construct(
        private readonly TenantContext $tenantContext,
        private readonly AuditLogger $auditLogger,
    ) {
    }

    /**
     * Sets the same TenantContext (and therefore the same database session
     * variable) any other resolution mechanism sets — "there is exactly one
     * way to hold a trainer-scoped write lock, no matter which mechanism
     * opened it" (specs/security-voter-designer-design.md).
     */
    public function openFor(Trainer $trainer, Account $actor): void
    {
        $this->tenantContext->activateFor($trainer);
        $this->openForTrainerId = $trainer->getId();
        $this->auditLogger->record($actor, 'administrative_scope.opened', 'Trainer', $trainer->getId(), $trainer, []);
    }

    public function close(): void
    {
        $this->openForTrainerId = null;
        $this->tenantContext->clear();
    }

    /**
     * The check is against the SPECIFIC adopted trainer, not merely "some
     * scope is open somewhere" — a scope opened for Trainer A must not
     * satisfy a voter check against Trainer B's event.
     */
    public function isOpenFor(Trainer $trainer): bool
    {
        return null !== $this->openForTrainerId && $this->openForTrainerId === $trainer->getId();
    }
}
