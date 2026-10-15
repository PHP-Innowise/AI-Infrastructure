<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlayerProfile>
 */
class PlayerProfileRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlayerProfile::class);
    }

    /**
     * AC-01-13: is this account itself a self-training player.
     */
    public function findOneForSelfAccount(Account $account): ?PlayerProfile
    {
        return $this->findOneBy(['selfAccount' => $account]);
    }

    public function add(PlayerProfile $player): void
    {
        $this->getEntityManager()->persist($player);
    }
}
