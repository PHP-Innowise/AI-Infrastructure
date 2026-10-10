<?php

declare(strict_types=1);

namespace App\Growth\Dto;

use App\Identity\Entity\PlayerProfile;

/**
 * AC-06-15: one row of the Top Referrers leaderboard.
 */
final readonly class TopReferrerRow
{
    public function __construct(
        public PlayerProfile $player,
        public int $totalReferrals,
        public int $totalConversions,
        public float $conversionRatePercent,
        public \DateTimeImmutable $lastReferralAt,
    ) {
    }
}
