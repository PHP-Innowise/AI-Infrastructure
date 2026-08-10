<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: their own login if they have one,
 * else the parent on record — a child usually has no login of their own.
 * Shared by SchedulingMailer (who to email) and RsvpService (who the payer
 * is for a payment-intent request), so the resolution rule lives in exactly
 * one place.
 */
final readonly class PlayerAccountResolver
{
    public function __construct(
        private ParentChildLinkRepository $parentChildLinks,
    ) {
    }

    public function resolve(PlayerProfile $player): ?Account
    {
        return $player->getSelfAccount()
            ?? $this->parentChildLinks->findByChildPlayer($player)?->getParentAccount();
    }
}
