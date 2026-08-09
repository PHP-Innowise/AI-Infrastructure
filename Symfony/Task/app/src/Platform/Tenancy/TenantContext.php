<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

use App\Platform\Entity\Trainer;
use Doctrine\DBAL\Connection;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Holds the active trainer for the current request, message or command, and
 * mirrors it into the PostgreSQL session variable that every Row-Level
 * Security policy reads.
 *
 * Tenancy layer 3. The rule that makes this layer worth having: **unresolved
 * means exception, never "all"**. RLS on its own fails closed *silently* — a
 * worker with no tenant would read zero rows forever and never complain. This
 * class converts that silence into a loud failure at the first trainer-scoped
 * query.
 *
 * @see specs/architect-architecture.md "Layer 3 — the mandatory tenant context"
 */
final class TenantContext
{
    /**
     * The PostgreSQL session variable every RLS policy compares against.
     * Namespaced so it cannot collide with a built-in setting.
     */
    public const SESSION_VARIABLE = 'app.current_trainer';

    private ?int $trainerId = null;

    public function __construct(
        private readonly Connection $connection,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * Establish the tenant. Writing the database session variable is not
     * optional bookkeeping: it is what makes every RLS policy evaluate, so it
     * happens in the same call that sets the in-memory value.
     */
    public function activate(int $trainerId): void
    {
        if ($trainerId <= 0) {
            throw new \InvalidArgumentException('A trainer id must be a positive integer.');
        }

        $this->trainerId = $trainerId;
        $this->writeSessionVariable((string) $trainerId);
        $this->enableDoctrineFilter($trainerId);
    }

    public function activateFor(Trainer $trainer): void
    {
        $id = $trainer->getId();

        if (null === $id) {
            throw new \LogicException('Cannot activate a tenant for an unpersisted Trainer.');
        }

        $this->activate($id);
    }

    /**
     * Tear the tenant down. Called between messages in the Messenger
     * middleware, so one message's tenant can never leak into the next.
     */
    public function clear(): void
    {
        $this->trainerId = null;
        $this->writeSessionVariable('');

        if ($this->entityManager->getFilters()->isEnabled(TenantFilter::NAME)) {
            $this->entityManager->getFilters()->disable(TenantFilter::NAME);
        }
    }

    public function isResolved(): bool
    {
        return null !== $this->trainerId;
    }

    public function getTrainerIdOrNull(): ?int
    {
        return $this->trainerId;
    }

    /**
     * The accessor trainer-scoped code should use. Throwing here is the whole
     * point of the layer: a caller that has forgotten to establish a tenant
     * finds out immediately, rather than quietly operating on an empty world.
     */
    public function requireTrainerId(): int
    {
        if (null === $this->trainerId) {
            throw new TenantNotResolvedException(
                'No tenant is resolved. Trainer-scoped data cannot be read or written without one.'
            );
        }

        return $this->trainerId;
    }

    private function writeSessionVariable(string $value): void
    {
        // set_config with is_local=false sets it for the session rather than
        // the transaction: the value must survive across the several
        // transactions a single request may open.
        $this->connection->executeStatement(
            'SELECT set_config(?, ?, false)',
            [self::SESSION_VARIABLE, $value],
        );
    }

    private function enableDoctrineFilter(int $trainerId): void
    {
        $filters = $this->entityManager->getFilters();

        $filter = $filters->isEnabled(TenantFilter::NAME)
            ? $filters->getFilter(TenantFilter::NAME)
            : $filters->enable(TenantFilter::NAME);

        $filter->setParameter('trainer_id', $trainerId);
    }
}
