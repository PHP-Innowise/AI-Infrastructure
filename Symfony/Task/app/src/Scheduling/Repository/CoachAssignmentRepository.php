<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Identity\Entity\CoachMembership;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<CoachAssignment>
 */
class CoachAssignmentRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, CoachAssignment::class);
    }

    public function findOneByEventAndCoach(Event $event, CoachMembership $coach): ?CoachAssignment
    {
        return $this->findOneBy(['event' => $event, 'coachMembership' => $coach]);
    }

    /**
     * The event's current, operative assignment — the most recently
     * (re)assigned non-declined row. See CoachAssignment's own docblock for
     * why "current" is derived this way rather than stored directly.
     */
    public function findCurrentForEvent(Event $event): ?CoachAssignment
    {
        /** @var list<CoachAssignment> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.event = :event')
            ->andWhere('a.status != :declined')
            ->setParameter('event', $event)
            ->setParameter('declined', CoachAssignment::STATUS_DECLINED)
            ->orderBy('a.assignedAt', 'DESC')
            ->setMaxResults(1)
            ->getQuery()
            ->getResult();

        return $rows[0] ?? null;
    }

    /**
     * @return list<CoachAssignment>
     */
    public function findAllNonDeclinedForEvent(Event $event): array
    {
        /** @var list<CoachAssignment> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.event = :event')
            ->andWhere('a.status != :declined')
            ->setParameter('event', $event)
            ->setParameter('declined', CoachAssignment::STATUS_DECLINED)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-02-34: "Events to Confirm" — this coach's own pending assignments,
     * within the active tenant.
     *
     * @return list<CoachAssignment>
     */
    public function findPendingForCoach(CoachMembership $coach): array
    {
        return $this->findByCoachAndStatus($coach, CoachAssignment::STATUS_PENDING);
    }

    /**
     * AC-02-35/"Assigned Sessions".
     *
     * @return list<CoachAssignment>
     */
    public function findConfirmedForCoach(CoachMembership $coach): array
    {
        return $this->findByCoachAndStatus($coach, CoachAssignment::STATUS_CONFIRMED);
    }

    /**
     * BR-02-15: overlap check for a proposed (start, end) — a coach cannot
     * be assigned to overlapping events. `$excludingEvent` lets a re-save of
     * the SAME event (e.g. editing its time) skip comparing against itself.
     *
     * @return list<CoachAssignment>
     */
    public function findOverlappingForCoach(
        CoachMembership $coach,
        \DateTimeImmutable $start,
        \DateTimeImmutable $end,
        ?Event $excludingEvent = null,
    ): array {
        $qb = $this->createQueryBuilder('a')
            ->join('a.event', 'e')
            ->andWhere('a.coachMembership = :coach')
            ->andWhere('a.status != :declined')
            ->andWhere('e.status = :active')
            ->andWhere('e.startsAt < :end')
            ->andWhere('e.endsAt > :start')
            ->setParameter('coach', $coach)
            ->setParameter('declined', CoachAssignment::STATUS_DECLINED)
            ->setParameter('active', Event::STATUS_ACTIVE)
            ->setParameter('start', $start)
            ->setParameter('end', $end);

        if (null !== $excludingEvent) {
            $qb->andWhere('e != :excluding')->setParameter('excluding', $excludingEvent);
        }

        /** @var list<CoachAssignment> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    /**
     * @return list<CoachAssignment>
     */
    private function findByCoachAndStatus(CoachMembership $coach, string $status): array
    {
        /** @var list<CoachAssignment> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.coachMembership = :coach')
            ->andWhere('a.status = :status')
            ->setParameter('coach', $coach)
            ->setParameter('status', $status)
            ->orderBy('a.assignedAt', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(CoachAssignment $assignment): void
    {
        $this->getEntityManager()->persist($assignment);
    }
}
