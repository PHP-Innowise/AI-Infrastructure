<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Platform\Entity\Trainer;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\DBAL\LockMode;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Event>
 */
class EventRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Event::class);
    }

    /**
     * AC-02-67/capacity checks convention (specs/database-designer-schema.md
     * "Conventions" — "Capacity checks"): row-locks the event for the
     * duration of the caller's transaction. Per the architecture's lock
     * ordering rule, this must be the LAST lock taken in any sequence that
     * also locks a token balance row (Epic-05) — see RsvpService.
     */
    public function lockForUpdate(int $eventId): ?Event
    {
        return $this->getEntityManager()->find(Event::class, $eventId, LockMode::PESSIMISTIC_WRITE);
    }

    /**
     * AC-02-1/US-02.01: the trainer's own Event Builder list, within the
     * active tenant.
     *
     * @return list<Event>
     */
    public function findAllForActiveTenant(): array
    {
        /** @var list<Event> $rows */
        $rows = $this->createQueryBuilder('e')
            ->orderBy('e.startsAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-02-19: the Training Calendar's own public+eligible pool, within the
     * active tenant. Eligibility (age/skill/gender) and private-invitation
     * filtering are applied by the caller (EventEligibilityChecker) since
     * they need data this repository does not own (a player's membership,
     * an invitation list).
     *
     * @return list<Event>
     */
    public function findUpcomingForActiveTenant(\DateTimeImmutable $now): array
    {
        /** @var list<Event> $rows */
        $rows = $this->createQueryBuilder('e')
            ->andWhere('e.status = :active')
            ->andWhere('e.startsAt >= :now')
            ->setParameter('active', Event::STATUS_ACTIVE)
            ->setParameter('now', $now)
            ->orderBy('e.startsAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-35: "Events This Week" (vs. last week) — events STARTING within
     * the given window, within the active tenant. The caller supplies both
     * this-week's and last-week's bounds and calls this twice; no "week"
     * concept is baked in here, matching the Quick View dashboard's own
     * "fixed windows" framing (specs/api-designer-spec.md:533).
     */
    public function countStartingBetween(\DateTimeImmutable $from, \DateTimeImmutable $to): int
    {
        $count = $this->createQueryBuilder('e')
            ->select('COUNT(e.id)')
            ->andWhere('e.startsAt >= :from')
            ->andWhere('e.startsAt < :to')
            ->setParameter('from', $from)
            ->setParameter('to', $to)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * AC-07-22..24/AC-02-55..56: Super Admin's Event Master tool reads
     * across every trainer, so it never goes through this repository at all
     * — see CrossTenantReadService. This repository stays tenant-scoped,
     * every method here included.
     */
    public function add(Event $event): void
    {
        $this->getEntityManager()->persist($event);
    }
}
