<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: the parent on record if this player
 * IS someone's registered child, else their own login — same "parent-link
 * first, self-account fallback" logic as
 * `App\Billing\Service\PlayerAccountResolver` /
 * `App\Scheduling\Service\PlayerAccountResolver` /
 * `App\Content\Service\PlayerAccountResolver`, deliberately duplicated here
 * rather than reused (see those classes' own docblocks for the module-
 * boundary reasoning: each module owns its own narrow copy of this
 * Identity-shaped concern). Growth needs this to resolve which account a
 * referral-reward token is credited to (AC-06-10: "added to their balance
 * with that specific trainer") — the same "who holds the wallet" question
 * Billing answers for a purchase.
 */
final readonly class PlayerAccountResolver
{
    public function __construct(
        private ParentChildLinkRepository $parentChildLinks,
    ) {
    }

    public function resolve(PlayerProfile $player): ?Account
    {
        return $this->parentChildLinks->findByChildPlayer($player)?->getParentAccount()
            ?? $player->getSelfAccount();
    }
}
