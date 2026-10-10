<?php

declare(strict_types=1);

namespace App\Growth\Dto;

/**
 * The platform-wide referral reward rule (BR-06-4), read from
 * `PlatformConfiguration` and interpreted here by Growth — the table itself
 * stays a generic key/value store; this is Growth's own typed view of its
 * three keys.
 *
 * Q-06.11 defaults: 1 referral required per 1 token awarded, no referee
 * welcome bonus. Q-06.12 default: a 30-day attribution window, configurable.
 */
final readonly class ReferralRule
{
    public function __construct(
        public int $referralsRequired,
        public int $tokensAwarded,
        public bool $refereeWelcomeBonus,
        public int $attributionWindowDays,
    ) {
    }
}
