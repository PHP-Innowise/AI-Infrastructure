<?php

declare(strict_types=1);

namespace App\Crm\Repository;

use App\Crm\Entity\PlayerNote;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlayerNote>
 */
class PlayerNoteRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlayerNote::class);
    }

    /**
     * AC-03-19/BR-03-10: the player detail's Notes section — chronological,
     * most recent first, general notes only.
     *
     * @return list<PlayerNote>
     */
    public function findGeneralForPlayer(PlayerProfile $player): array
    {
        return $this->findTypedForPlayer($player, PlayerNote::TYPE_GENERAL);
    }

    /**
     * AC-03-46..49: a coach's own past feedback for this player (session
     * notes authored by the given coach account), most recent first — the
     * source list `SessionFeedbackType`'s dropdown offers, and the "past
     * feedback from this coach" the coach-scoped detail view shows.
     *
     * @return list<PlayerNote>
     */
    public function findSessionNotesForPlayerByAuthor(PlayerProfile $player, Account $author): array
    {
        /** @var list<PlayerNote> $rows */
        $rows = $this->createQueryBuilder('n')
            ->andWhere('n.player = :player')
            ->andWhere('n.noteType = :type')
            ->andWhere('n.createdByAccount = :author')
            ->setParameter('player', $player)
            ->setParameter('type', PlayerNote::TYPE_SESSION)
            ->setParameter('author', $author)
            ->orderBy('n.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-20/AC-03-48: every session/event note for this player —
     * trainer per-event notes and coach session feedback alike — for the
     * player detail's Event History section and the trainer's own view of
     * coach feedback (AC-03-32).
     *
     * @return list<PlayerNote>
     */
    public function findSessionNotesForPlayer(PlayerProfile $player): array
    {
        return $this->findTypedForPlayer($player, PlayerNote::TYPE_SESSION);
    }

    /**
     * @return list<PlayerNote>
     */
    public function findForEventAndPlayer(Event $event, PlayerProfile $player): array
    {
        /** @var list<PlayerNote> $rows */
        $rows = $this->createQueryBuilder('n')
            ->andWhere('n.event = :event')
            ->andWhere('n.player = :player')
            ->setParameter('event', $event)
            ->setParameter('player', $player)
            ->orderBy('n.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * @return list<PlayerNote>
     */
    private function findTypedForPlayer(PlayerProfile $player, string $noteType): array
    {
        /** @var list<PlayerNote> $rows */
        $rows = $this->createQueryBuilder('n')
            ->andWhere('n.player = :player')
            ->andWhere('n.noteType = :type')
            ->setParameter('player', $player)
            ->setParameter('type', $noteType)
            ->orderBy('n.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(PlayerNote $note): void
    {
        $this->getEntityManager()->persist($note);
    }

    public function remove(PlayerNote $note): void
    {
        $this->getEntityManager()->remove($note);
    }
}
