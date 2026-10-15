<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Entity\Coupon;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;

/**
 * Q-06.10's settled default: coupon eligibility is a per-coupon setting
 * (`any_player` or `new_players_only`) — the "who decides" half of the
 * question. The epic never defines what "new" means operationally, and
 * remains an open question in every spec that touches it.
 *
 * This codebase's own reading, applied consistently and documented rather
 * than guessed silently: a player counts as "new" to a trainer if they have
 * never redeemed any coupon with that trainer before — the one prior-
 * activity signal Growth owns outright without reaching into Billing's,
 * Scheduling's, or Content's purchase history directly (the module map
 * authorizes Growth to call Billing only "to grant a reward entry," not for
 * general reads). A trainer wanting a stricter "never purchased anything at
 * all" rule is not expressible with this interpretation — recorded as a
 * conflict/judgment call in the coder's final report, not silently resolved
 * as if Q-06.10 were actually settled.
 */
final readonly class CouponEligibilityChecker
{
    public function __construct(
        private CouponRedemptionRepository $redemptions,
    ) {
    }

    public function isEligible(Coupon $coupon, PlayerProfile $player, Trainer $trainer): bool
    {
        if (Coupon::ELIGIBILITY_ANY_PLAYER === $coupon->getEligibility()) {
            return true;
        }

        return !$this->redemptions->hasAnyRedemptionForPlayer($trainer, $player);
    }
}
