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

    /**
     * AC-03-43/64: the coach-scoped player list — every active membership
     * for a caller-supplied set of player ids (CoachVisibilityService's own
     * reachable set), within the active tenant. An empty id list means an
     * empty roster (AC-03-64: "a coach assigned to zero events sees zero
     * players"), not "no filter."
     *
     * @param list<int> $playerIds
     *
     * @return list<PlayerTrainerMembership>
     */
    public function findActiveForPlayerIds(array $playerIds): array
    {
        if ([] === $playerIds) {
            return [];
        }

        /** @var list<PlayerTrainerMembership> $rows */
        $rows = $this->createQueryBuilder('m')
            ->andWhere('m.status = :active')
            ->andWhere('IDENTITY(m.player) IN (:playerIds)')
            ->setParameter('active', PlayerTrainerMembership::STATUS_ACTIVE)
            ->setParameter('playerIds', $playerIds)
            ->orderBy('m.joinedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-39/BR-03-23: "5 new players joined via ShareLink" this week —
     * `joined_at` doubles as "when this association was created" (BR-01-12's
     * reactivate-in-place model means a re-join also updates it), within the
     * active tenant.
     */
    public function countNewViaShareLinkBetween(\DateTimeImmutable $from, \DateTimeImmutable $to): int
    {
        $count = $this->createQueryBuilder('m')
            ->select('COUNT(m.id)')
            ->andWhere('m.source = :source')
            ->andWhere('m.joinedAt >= :from')
            ->andWhere('m.joinedAt < :to')
            ->setParameter('source', PlayerTrainerMembership::SOURCE_SHARELINK)
            ->setParameter('from', $from)
            ->setParameter('to', $to)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    public function add(PlayerTrainerMembership $membership): void
    {
        $this->getEntityManager()->persist($membership);
    }
}
