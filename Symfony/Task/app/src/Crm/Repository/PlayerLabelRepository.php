<?php

declare(strict_types=1);

namespace App\Crm\Repository;

use App\Crm\Entity\Label;
use App\Crm\Entity\PlayerLabel;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlayerLabel>
 */
class PlayerLabelRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlayerLabel::class);
    }

    /**
     * AC-03-28: the player detail's Labels section.
     *
     * @return list<PlayerLabel>
     */
    public function findForPlayer(PlayerProfile $player): array
    {
        /** @var list<PlayerLabel> $rows */
        $rows = $this->createQueryBuilder('pl')
            ->andWhere('pl.player = :player')
            ->setParameter('player', $player)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function findOneByPlayerAndLabel(PlayerProfile $player, Label $label): ?PlayerLabel
    {
        return $this->findOneBy(['player' => $player, 'label' => $label]);
    }

    /**
     * The reverse of findForPlayer(): every player CURRENTLY carrying this
     * label. Added for Content (Epic-04) AC-04-13/BR-04-13's "assign by
     * label group" — the architecture's module map explicitly permits
     * Content to read Crm's labels for group assignment
     * (specs/architect-architecture.md "Module map": Content "may call...
     * Crm (read labels for group assignment)"), through this repository,
     * never by Content writing or reimplementing label logic of its own.
     *
     * @return list<int>
     */
    public function findPlayerIdsForLabel(Label $label): array
    {
        $rows = $this->createQueryBuilder('pl')
            ->select('IDENTITY(pl.player) AS player_id')
            ->andWhere('pl.label = :label')
            ->setParameter('label', $label)
            ->getQuery()
            ->getScalarResult();

        return array_values(array_map(static fn (array $row): int => (int) $row['player_id'], $rows));
    }

    public function add(PlayerLabel $playerLabel): void
    {
        $this->getEntityManager()->persist($playerLabel);
    }

    public function remove(PlayerLabel $playerLabel): void
    {
        $this->getEntityManager()->remove($playerLabel);
    }
}
