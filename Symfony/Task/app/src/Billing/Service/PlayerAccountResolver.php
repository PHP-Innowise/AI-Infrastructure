<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: the parent on record if this
 * player IS someone's registered child, else their own login — BR-05-2's
 * "parent-trainer" pair. Deliberately duplicates
 * `App\Scheduling\Service\PlayerAccountResolver`'s /
 * `App\Content\Service\PlayerAccountResolver`'s exact logic rather than
 * reusing either: each module owns its own narrow copy of this
 * Identity-shaped concern (see `App\Content\Service\PlayerAccountResolver`'s
 * own docblock for the module-boundary reasoning, identical here).
 *
 * Parent-link-first, not self-account-first: a child may hold their OWN
 * login (`giveChildOwnLogin()`'s own scenario, matching
 * `ChildApprovalVoter::voteBypass()`'s "actor is the child's own login"
 * branch) purely for AUTHENTICATION, while remaining financially a minor
 * whose parent holds the wallet — AC-05-6 is explicit that such a child
 * "sees the PARENT's balance," and AC-05-9's "a parent RSVPing a child...
 * not a child-specific balance" states the same fact from the other
 * direction. A player with a self-account and NO `ParentChildLink` (an
 * independent adult player, e.g. the fixture "Pat") still resolves to
 * their own account — the link lookup simply finds nothing for them, and
 * the self-account fallback applies. Getting this order backwards was a
 * genuine bug caught by AC-05-6's own test: a child with their own login
 * saw a balance of 0 (their own, un-funded account) instead of the
 * parent's.
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
