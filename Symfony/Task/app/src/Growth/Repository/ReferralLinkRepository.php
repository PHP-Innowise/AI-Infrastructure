<?php

declare(strict_types=1);

namespace App\Growth\Repository;

use App\Growth\Entity\ReferralLink;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ReferralLink>
 */
class ReferralLinkRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ReferralLink::class);
    }

    public function findOneByTrainerAndPlayer(Trainer $trainer, PlayerProfile $player): ?ReferralLink
    {
        return $this->findOneBy(['trainer' => $trainer, 'player' => $player]);
    }

    /**
     * The referrer side of a referral link — resolved by (trainer, player)
     * id pair from the public `/join/{trainerSlug}/{playerId}` route, before
     * any tenant/RLS context is necessarily active for THIS player's own
     * row (the visiting friend is anonymous). Used only by the public join
     * controller, which activates the tenant from the trainer slug first.
     */
    public function findOneByTrainerAndPlayerId(Trainer $trainer, int $playerId): ?ReferralLink
    {
        return $this->findOneBy(['trainer' => $trainer, 'player' => $playerId]);
    }

    public function add(ReferralLink $referralLink): void
    {
        $this->getEntityManager()->persist($referralLink);
    }
}
