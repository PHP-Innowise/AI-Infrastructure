<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlayerTrainerMembership>
 */
class PlayerTrainerMembershipRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlayerTrainerMembership::class);
    }

    /**
     * BR-01-12: re-joining reactivates the same row rather than inserting a
     * duplicate — this is the lookup that makes that possible, regardless of
     * the row's current status.
     */
    public function findOneByTrainerAndPlayer(Trainer $trainer, PlayerProfile $player): ?PlayerTrainerMembership
    {
        return $this->findOneBy(['trainer' => $trainer, 'player' => $player]);
    }

    /**
     * AC-01-15/AC-01-22: every trainer a given player is (still) actively
     * associated with.
     *
     * @return list<PlayerTrainerMembership>
     */
    public function findActiveForPlayer(PlayerProfile $player): array
    {
        /** @var list<PlayerTrainerMembership> $rows */
        $rows = $this->createQueryBuilder('m')
            ->andWhere('m.player = :player')
            ->andWhere('m.status = :active')
            ->setParameter('player', $player)
            ->setParameter('active', PlayerTrainerMembership::STATUS_ACTIVE)
            ->orderBy('m.joinedAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * The CRM roster for the active tenant (RLS + the Doctrine filter both
     * apply automatically).
     *
     * @return list<PlayerTrainerMembership>
     */
    public function findActiveForActiveTenant(): array
    {
        /** @var list<PlayerTrainerMembership> $rows */
        $rows = $this->createQueryBuilder('m')
            ->andWhere('m.status = :active')
            ->setParameter('active', PlayerTrainerMembership::STATUS_ACTIVE)
            ->orderBy('m.joinedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(PlayerTrainerMembership $membership): void
    {
        $this->getEntityManager()->persist($membership);
    }
}
