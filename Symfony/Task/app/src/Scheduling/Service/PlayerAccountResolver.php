<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\ParentChildLinkRepository;

/**
 * The account responsible for a player: the parent on record if this
 * player IS someone's registered child, else their own login. Shared by
 * SchedulingMailer (who to email) and RsvpService (who the payer is for a
 * payment-intent request), so the resolution rule lives in exactly one
 * place.
 *
 * Parent-link-first, not self-account-first — see
 * `App\Billing\Service\PlayerAccountResolver`'s own docblock (this
 * module's identical copy) for why a child who holds their own login for
 * AUTHENTICATION (`giveChildOwnLogin()`) must still resolve to the
 * parent's account for money purposes (AC-05-6, AC-05-9), and why an
 * independent adult player (a self-account with no `ParentChildLink`)
 * still resolves to themselves either way.
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
