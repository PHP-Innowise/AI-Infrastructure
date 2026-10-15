<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: the parent on record if this
 * player IS someone's registered child, else their own login.
 * Deliberately duplicates `App\Scheduling\Service\PlayerAccountResolver`'s
 * exact logic rather than reusing that class — architect-architecture.md
 * "Module map" lists Content as allowed to call Platform, Identity, Billing
 * and Crm, but NOT Scheduling, so this tiny resolver is Content's own copy
 * of an Identity-shaped concern, not a cross-module dependency.
 *
 * Parent-link-first, not self-account-first — see
 * `App\Billing\Service\PlayerAccountResolver`'s own docblock (also
 * duplicated from this exact shape) for why a child who holds their own
 * login for AUTHENTICATION (`giveChildOwnLogin()`) must still resolve to
 * the parent's account for money purposes.
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
