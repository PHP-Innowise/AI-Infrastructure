<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Entity\ReferralLink;
use App\Growth\Repository\ReferralLinkRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * AC-06-1..3: every player automatically has a referral link with no
 * "generate" action — provisioned lazily, on first read, exactly like
 * `TrainerBillingSettingsRepository::getOrCreateForTrainer()` provisions a
 * trainer's billing settings. A player training with several trainers gets a
 * separate link per trainer (AC-06-3) because this is keyed on
 * (trainer, player), not on player alone.
 */
final readonly class ReferralLinkService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ReferralLinkRepository $referralLinks,
    ) {
    }

    public function linkFor(Trainer $trainer, PlayerProfile $player): ReferralLink
    {
        $existing = $this->referralLinks->findOneByTrainerAndPlayer($trainer, $player);

        if (null !== $existing) {
            return $existing;
        }

        $link = new ReferralLink($trainer, $player);
        $this->referralLinks->add($link);
        $this->entityManager->flush();

        return $link;
    }
}
