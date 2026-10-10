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
     * The pre-check rejection leaves the closure as a return value and is
     * thrown outside it, so a flag the trainer already has open stays an
     * ordinary form error instead of closing the caller's EntityManager —
     * `LabelService::create()` carries the full reasoning, including why the
     * unique-index race below is the one branch that legitimately does not
     * get the same treatment.
     *
     * @throws DuplicateActiveFlagException
     */
    public function apply(PlayerTrainerMembership $membership, string $flagType, Account $actor, ?string $note): PlayerFlag
    {
        try {
            $outcome = $this->entityManager->wrapInTransaction(function () use ($membership, $flagType, $actor, $note): PlayerFlag|DuplicateActiveFlagException {
                // BR-03-8 pre-check: reapplication is only legitimate once the
                // earlier instance is resolved. uniq_player_flag_active_type is
                // the authoritative backstop under concurrency, caught below.
                if (null !== $this->flags->findOneActiveByPlayerAndType($membership->getPlayer(), $flagType)) {
                    return DuplicateActiveFlagException::forType($flagType);
                }

                $flag = new PlayerFlag($membership->getTrainer(), $membership->getPlayer(), $flagType, $actor, $note);
                $this->flags->add($flag);
                $this->entityManager->flush();

                return $flag;
            });
        } catch (UniqueConstraintViolationException) {
            throw DuplicateActiveFlagException::forType($flagType);
        }

        if ($outcome instanceof DuplicateActiveFlagException) {
            throw $outcome;
        }

        return $outcome;
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
