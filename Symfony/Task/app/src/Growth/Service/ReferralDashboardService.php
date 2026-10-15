<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Dto\ReferralOverviewMetrics;
use App\Growth\Dto\TopReferrerRow;
use App\Growth\Entity\Referral;
use App\Growth\Repository\ReferralRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Platform\Entity\Trainer;

/**
 * US-06.04: the trainer referral dashboard's read model —
 * `TrainerReferralDashboardController`'s only collaborator, matching
 * `Crm\Service\QuickViewDashboardService`'s own "repository returns raw
 * counts, service shapes the view model" split.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-13..16
 */
final readonly class ReferralDashboardService
{
    public function __construct(
        private ReferralRepository $referrals,
        private PlayerProfileRepository $playerProfiles,
    ) {
    }

    public function overviewMetrics(Trainer $trainer, \DateTimeImmutable $monthStart, \DateTimeImmutable $monthEnd): ReferralOverviewMetrics
    {
        $totalReferrals = $this->referrals->countRegisteredBetween($trainer, $monthStart, $monthEnd);
        $totalConversions = $this->referrals->countConvertedBetween($trainer, $monthStart, $monthEnd);

        return new ReferralOverviewMetrics(
            totalReferralsThisMonth: $totalReferrals,
            totalConversionsThisMonth: $totalConversions,
            conversionRatePercent: $this->rate($totalConversions, $totalReferrals),
            totalReferralRevenueMinorUnits: $this->referrals->sumFirstPurchaseRevenue($trainer),
        );
    }

    /**
     * @return list<TopReferrerRow>
     */
    public function topReferrers(Trainer $trainer, int $limit = 10): array
    {
        $rows = [];

        foreach ($this->referrals->topReferrers($trainer, $limit) as $row) {
            $player = $this->playerProfiles->find($row['referrerPlayerId']);

            if (null === $player) {
                continue;
            }

            $rows[] = new TopReferrerRow(
                player: $player,
                totalReferrals: $row['totalReferrals'],
                totalConversions: $row['totalConversions'],
                conversionRatePercent: $this->rate($row['totalConversions'], $row['totalReferrals']),
                lastReferralAt: $row['lastReferralAt'],
            );
        }

        return $rows;
    }

    /**
     * AC-06-16: Referral Activity Log, newest first.
     *
     * @return list<Referral>
     */
    public function activityLog(Trainer $trainer, int $limit = 50): array
    {
        return $this->referrals->activityLog($trainer, $limit);
    }

    private function rate(int $numerator, int $denominator): float
    {
        return $denominator > 0 ? round(100 * $numerator / $denominator, 1) : 0.0;
    }
}
