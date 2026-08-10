<?php

declare(strict_types=1);

namespace App\Crm\Service;

use App\Crm\Entity\PlayerFlag;
use App\Crm\Exception\DuplicateActiveFlagException;
use App\Crm\Repository\PlayerFlagRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Apply and resolve system-defined flags (US-03.04). BR-03-6: exactly the 8
 * fixed flags — enforced by `PlayerFlag`'s own constructor guard, not
 * restated here.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-6/7/8, AC-03-15..18
 */
final readonly class PlayerFlagService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlayerFlagRepository $flags,
    ) {
    }

    /**
     * @throws DuplicateActiveFlagException
     */
    public function apply(PlayerTrainerMembership $membership, string $flagType, Account $actor, ?string $note): PlayerFlag
    {
        return $this->entityManager->wrapInTransaction(function () use ($membership, $flagType, $actor, $note): PlayerFlag {
            // BR-03-8 pre-check: reapplication is only legitimate once the
            // earlier instance is resolved. uniq_player_flag_active_type is
            // the authoritative backstop under concurrency, caught below.
            if (null !== $this->flags->findOneActiveByPlayerAndType($membership->getPlayer(), $flagType)) {
                throw DuplicateActiveFlagException::forType($flagType);
            }

            $flag = new PlayerFlag($membership->getTrainer(), $membership->getPlayer(), $flagType, $actor, $note);
            $this->flags->add($flag);

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw DuplicateActiveFlagException::forType($flagType);
            }

            return $flag;
        });
    }

    /**
     * BR-03-8: hides from the active view; history is preserved on the same
     * row (see PlayerFlag::resolve()).
     */
    public function resolve(PlayerFlag $flag, Account $resolvedBy): void
    {
        $flag->resolve($resolvedBy);
        $this->entityManager->flush();
    }
}
