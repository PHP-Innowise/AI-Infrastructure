<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerProfileRepository;
use Symfony\Component\HttpFoundation\Request;

/**
 * "The current player context" — the acting player themselves, or, for a
 * parent, whichever child was last selected via
 * `identity_portal_context_child_switch` (FamilyController). Every
 * player-portal controller that needs to know which PlayerProfile is
 * acting resolves it the same way; extracted here so Epic-02's several new
 * portal controllers (calendar, RSVP) do not each duplicate the session-key
 * lookup `PortalAvailabilityController::currentPlayerContext()` already
 * established inline.
 *
 * Session key intentionally matches FamilyController's own
 * `'current_player_context_id'` exactly — this is the same switch, read by
 * a second consumer, not a new one.
 */
final readonly class PlayerContextResolver
{
    private const SESSION_KEY = 'current_player_context_id';

    public function __construct(
        private PlayerProfileRepository $playerProfiles,
    ) {
    }

    public function resolve(Request $request, Account $account): PlayerProfile
    {
        $contextId = $request->getSession()->get(self::SESSION_KEY);

        if (\is_int($contextId)) {
            $player = $this->playerProfiles->find($contextId);

            if (null !== $player) {
                return $player;
            }
        }

        return $this->playerProfiles->findOneForSelfAccount($account)
            ?? throw new \RuntimeException('No player profile for this account.');
    }
}
