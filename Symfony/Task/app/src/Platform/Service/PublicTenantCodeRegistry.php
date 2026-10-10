<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\PublicTenantCodeRepository;

/**
 * The single writer of `PublicTenantCode` rows (schema doc, "public_tenant_code"
 * — "Single writer"). `Identity` calls `issue()` when a ShareLink is created,
 * in the same transaction that persists the ShareLink itself, so the mapping
 * can never outlive (or predate) the row it points at.
 *
 * A `Platform`-owned service on purpose: `Platform` "must not depend on any
 * other module" (architect-architecture.md, Module map), so this class holds
 * no reference to `App\Identity\Entity\ShareLink` — callers pass a bare
 * `kind` string and `referenceId` int instead.
 *
 * @see specs/database-designer-schema.md "`public_tenant_code`"
 */
final readonly class PublicTenantCodeRegistry
{
    public function __construct(
        private PublicTenantCodeRepository $repository,
    ) {
    }

    public function issue(string $code, Trainer $trainer, string $kind, int $referenceId): PublicTenantCode
    {
        $entry = new PublicTenantCode($code, $trainer, $kind, $referenceId);
        $this->repository->add($entry);

        return $entry;
    }

    public function revoke(string $kind, int $referenceId): void
    {
        $entry = $this->repository->findOneByReference($kind, $referenceId);

        if (null !== $entry) {
            $this->repository->remove($entry);
        }
    }
}
