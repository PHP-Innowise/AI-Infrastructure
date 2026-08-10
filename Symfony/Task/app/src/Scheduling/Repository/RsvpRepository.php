<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Rsvp>
 */
class RsvpRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Rsvp::class);
    }

    /**
     * Any row for this (event, player) pair, regardless of status — there
     * is at most one, ever, per BR-02-7's unique constraint (see Rsvp's own
     * reactivate() docblock). Callers deciding whether to reuse/reactivate
     * an existing row want this; callers deciding "is this player
     * currently registered" want findActiveOneByEventAndPlayer() instead.
     */
    public function findOneByEventAndPlayer(Event $event, PlayerProfile $player): ?Rsvp
    {
        return $this->findOneBy(['event' => $event, 'player' => $player]);
    }

    /**
     * AC-02-28/BR-02-7: "already registered" means an active (non-canceled)
     * row — a player who canceled must be able to RSVP again. Used by the
     * duplicate check, the voter's own pre-check, and the portal's
     * "already registered" display alike, so all three agree.
     */
    public function findActiveOneByEventAndPlayer(Event $event, PlayerProfile $player): ?Rsvp
    {
        $rsvp = $this->findOneByEventAndPlayer($event, $player);

        return null !== $rsvp && !$rsvp->isCanceled() ? $rsvp : null;
    }

    /**
     * AC-02-67: the authoritative capacity count — every status that holds a
     * spot (Rsvp::CAPACITY_HOLDING_STATUSES), counted fresh, never cached
     * (specs/database-designer-schema.md "Conventions" — "Capacity checks").
     * Callers needing the locked, authoritative version call this only after
     * EventRepository::lockForUpdate() inside the same transaction; an
     * unlocked call (e.g. from a voter's fail-fast pre-check) is explicitly
     * non-authoritative per architect-architecture.md "Lock ordering".
     */
    public function countHeld(Event $event): int
    {
        $count = $this->createQueryBuilder('r')
            ->select('COUNT(r.id)')
            ->andWhere('r.event = :event')
            ->andWhere('r.status IN (:statuses)')
            ->setParameter('event', $event)
            ->setParameter('statuses', Rsvp::CAPACITY_HOLDING_STATUSES)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    /**
     * AC-02-43/44: the RSVP List tab.
     *
     * @return list<Rsvp>
     */
    public function findForEvent(Event $event): array
    {
        /** @var list<Rsvp> $rows */
        $rows = $this->createQueryBuilder('r')
            ->andWhere('r.event = :event')
            ->andWhere('r.status != :canceled')
            ->setParameter('event', $event)
            ->setParameter('canceled', Rsvp::STATUS_CANCELED)
            ->orderBy('r.requestedAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * US-02.06/07: a player's own reservations, within the active tenant.
     *
     * @return list<Rsvp>
     */
    public function findForPlayer(PlayerProfile $player): array
    {
        /** @var list<Rsvp> $rows */
        $rows = $this->createQueryBuilder('r')
            ->andWhere('r.player = :player')
            ->setParameter('player', $player)
            ->orderBy('r.requestedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-02-63/architect-architecture.md "Subscriptions are entitlements,
     * not ledger entries": entitlement-covered advance bookings on a given
     * calendar date, for the "1 per day in advance" limit. Not called from
     * anywhere in Epic-02 — SubscriptionEntitlement/EntitlementCoverage do
     * not exist until Epic-05 — kept here only as the natural extension
     * point so Epic-05 does not need to touch this repository's shape, and
     * cited by the skipped test that records why AC-02-63 cannot be
     * verified yet.
     *
     * @return list<Rsvp>
     */
    public function findConfirmedForPlayerOnDate(PlayerProfile $player, \DateTimeImmutable $dayStart, \DateTimeImmutable $dayEnd): array
    {
        /** @var list<Rsvp> $rows */
        $rows = $this->createQueryBuilder('r')
            ->join('r.event', 'e')
            ->andWhere('r.player = :player')
            ->andWhere('r.status = :confirmed')
            ->andWhere('e.startsAt >= :dayStart')
            ->andWhere('e.startsAt < :dayEnd')
            ->setParameter('player', $player)
            ->setParameter('confirmed', Rsvp::STATUS_CONFIRMED)
            ->setParameter('dayStart', $dayStart)
            ->setParameter('dayEnd', $dayEnd)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(Rsvp $rsvp): void
    {
        $this->getEntityManager()->persist($rsvp);
    }
}
