<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

use Doctrine\ORM\Mapping\ClassMetadata;
use Doctrine\ORM\Query\Filter\SQLFilter;

/**
 * Tenancy layer 2: appends `trainer_id = :trainer_id` to every query over an
 * entity marked #[TrainerScoped].
 *
 * Kept alongside Row-Level Security rather than replaced by it, because
 * neither is a superset of the other. The filter survives a misconfigured
 * database role; RLS survives a forgotten request listener. They fail in
 * different directions, which is the point.
 *
 * @see specs/architect-architecture.md Decisions, "Doctrine filter retained alongside RLS"
 */
final class TenantFilter extends SQLFilter
{
    public const NAME = 'tenant';

    public function addFilterConstraint(ClassMetadata $targetEntity, string $targetTableAlias): string
    {
        if ([] === $targetEntity->getReflectionClass()->getAttributes(TrainerScoped::class)) {
            return '';
        }

        try {
            $trainerId = $this->getParameter('trainer_id');
        } catch (\InvalidArgumentException) {
            // The filter is enabled but no tenant is set. Returning an
            // unsatisfiable predicate rather than an empty string keeps the
            // filter fail-closed: a bug that enables it without a tenant
            // yields nothing, never everything.
            return sprintf('%s.trainer_id IS NULL AND %s.trainer_id IS NOT NULL', $targetTableAlias, $targetTableAlias);
        }

        return sprintf('%s.trainer_id = %s', $targetTableAlias, $trainerId);
    }
}
