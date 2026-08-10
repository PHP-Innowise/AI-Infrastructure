<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: their own login if they have one,
 * else the parent on record. Deliberately duplicates
 * `App\Scheduling\Service\PlayerAccountResolver`'s exact logic rather than
 * reusing that class — architect-architecture.md "Module map" lists Content
 * as allowed to call Platform, Identity, Billing and Crm, but NOT
 * Scheduling, so this tiny resolver is Content's own copy of an
 * Identity-shaped concern, not a cross-module dependency.
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
