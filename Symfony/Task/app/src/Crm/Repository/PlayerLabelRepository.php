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

    public function add(PlayerLabel $playerLabel): void
    {
        $this->getEntityManager()->persist($playerLabel);
    }

    public function remove(PlayerLabel $playerLabel): void
    {
        $this->getEntityManager()->remove($playerLabel);
    }
}
