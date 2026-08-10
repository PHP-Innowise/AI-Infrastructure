<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\EventInvitation;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<EventInvitation>
 */
class EventInvitationRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, EventInvitation::class);
    }

    /**
     * BR-02-6: is this player invited to this private event.
     */
    public function isPlayerInvited(Event $event, PlayerProfile $player): bool
    {
        return null !== $this->findOneBy(['event' => $event, 'player' => $player]);
    }

    /**
     * @return list<EventInvitation>
     */
    public function findForEvent(Event $event): array
    {
        /** @var list<EventInvitation> $rows */
        $rows = $this->createQueryBuilder('i')
            ->andWhere('i.event = :event')
            ->setParameter('event', $event)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-02-19: every private event this player is invited to, within the
     * active tenant.
     *
     * @return list<int> event ids
     */
    public function findInvitedEventIdsForPlayer(PlayerProfile $player): array
    {
        /** @var list<array{eventId: int|string}> $rows */
        $rows = $this->createQueryBuilder('i')
            ->select('IDENTITY(i.event) AS eventId')
            ->andWhere('i.player = :player')
            ->setParameter('player', $player)
            ->getQuery()
            ->getArrayResult();

        return array_map(static fn (array $r): int => (int) $r['eventId'], $rows);
    }

    public function add(EventInvitation $invitation): void
    {
        $this->getEntityManager()->persist($invitation);
    }
}
