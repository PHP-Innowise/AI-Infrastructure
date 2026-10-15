<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<CoachMembership>
 */
class CoachMembershipRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, CoachMembership::class);
    }

    /**
     * AC-01-40: the trainer's Coaches list, within the active tenant.
     *
     * @return list<CoachMembership>
     */
    public function findAllForActiveTenant(): array
    {
        /** @var list<CoachMembership> $rows */
        $rows = $this->createQueryBuilder('c')
            ->orderBy('c.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * Same-tenant only — RLS means this can never see another trainer's rows,
     * by construction. Useful for idempotent invite handling ("has this
     * account already got a membership row here"), but this is NOT where
     * BR-01-11's cross-tenant exclusivity is enforced: that job belongs to
     * `uniq_coach_membership_active_account`, a unique partial index with no
     * `trainer_id` in its key, which is reachable precisely because Postgres
     * unique-index enforcement is not filtered by RLS. `MembershipService`
     * attempts the insert and translates that index's violation into
     * `CoachAlreadyActiveElsewhereException` — see its docblock.
     */
    public function findOneForAccountInActiveTenant(Account $account): ?CoachMembership
    {
        return $this->findOneBy(['account' => $account]);
    }

    public function add(CoachMembership $membership): void
    {
        $this->getEntityManager()->persist($membership);
    }
}
