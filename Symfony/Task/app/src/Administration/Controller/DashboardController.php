<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use App\Identity\Repository\AccountRepository;
use App\Platform\Service\CrossTenantReadService;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-07.01: the Super Admin operational dashboard — AC-07-1..7, AC-07-17.
 *
 * **No financial figure anywhere on this page, by construction** (AC-07-7):
 * nothing here calls `StripeReportingReader` or sums a `payment_record`
 * amount; the only Stripe-adjacent thing on this screen is a link-out
 * (`administration_stripe_dashboard_link`), never a number.
 *
 * **What the date-range selector (AC-07-5) actually drives**: the epic
 * names three DIFFERENT fixed windows in the same breath as the selector —
 * "new users this WEEK", "new users this MONTH", a "30-DAY growth chart"
 * (AC-07-2), and BR-07-9's own fixed 30-day top-performer ranking — none of
 * which reads as "whichever range the selector currently holds." This
 * implementation therefore binds the selector to the one block that names
 * no specific number of days in its own criterion text: Session Metrics
 * (AC-07-3, "sessions this week..." read as this section's own DEFAULT,
 * not a hard requirement distinct from the selector sitting right next to
 * it in the same user story). Every other metric keeps its own explicitly
 * stated fixed window regardless of `?range=`. Recorded as a judgment call
 * on an internally ambiguous requirement, not an invented one — the
 * observable requirement ("a 7/30/90 selector defaulting to 30 exists and
 * affects the dashboard") holds either way.
 *
 * Orchestrates several read-only sources directly rather than through an
 * extra service layer, matching `EventMasterController`/`UsersController`'s
 * own established shape for this module — Administration's controllers are
 * themselves the read-composition layer over other modules' services here,
 * not a thin pass-through to one.
 *
 * @see specs/api-designer-spec.md "Administration module" — administration_dashboard
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-1..7, BR-07-7..9
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class DashboardController extends AbstractController
{
    /**
     * @var list<int>
     */
    private const ALLOWED_RANGES = [7, 30, 90];

    private const GROWTH_CHART_DAYS = 30;

    public function __construct(
        private readonly AccountRepository $accounts,
        private readonly PlatformSubscriptionRepository $platformSubscriptions,
        private readonly CrossTenantReadService $crossTenantReads,
    ) {
    }

    #[Route('/super-admin/dashboard', name: 'administration_dashboard', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $range = $this->rangeFromRequest($request);
        $now = new \DateTimeImmutable();
        $rangeStart = $now->modify(sprintf('-%d days', $range));
        $weekStart = $now->modify('-7 days');
        $lastWeekStart = $now->modify('-14 days');
        $monthStart = $now->modify('-30 days');
        $actor = $this->actor();

        return $this->render('administration/dashboard.html.twig', [
            'range' => $range,
            'allowedRanges' => self::ALLOWED_RANGES,

            // AC-07-2: User Metrics.
            'totalTrainers' => $this->platformSubscriptions->countActive(),
            'totalPlayers' => $this->accounts->countByRoleAndStatus(AccountRole::Player, AccountStatus::Active),
            'totalCoaches' => $this->accounts->countByRoleAndStatus(AccountRole::Coach, AccountStatus::Active),
            'newUsersThisWeek' => $this->accounts->countCreatedBetween($weekStart, $now),
            'newUsersThisMonth' => $this->accounts->countCreatedBetween($monthStart, $now),
            'growthTrendPercent' => $this->growthTrendPercent($weekStart, $now, $lastWeekStart, $weekStart),
            'growthChart' => $this->growthChart($now),

            // AC-07-3: Session Metrics — driven by the selector, see this
            // class's own docblock.
            'sessionMetrics' => $this->crossTenantReads->dashboardSessionMetrics($actor, $rangeStart, $now),

            // AC-07-4: Top Performers — BR-07-9's own fixed 30-day window.
            'mostActiveTrainers' => $this->crossTenantReads->mostActiveTrainers($actor, 10, $now),
            'topPlayers' => $this->crossTenantReads->topPlayersSystemWide(10, $now, 30),
        ]);
    }

    private function rangeFromRequest(Request $request): int
    {
        $range = (int) $request->query->get('range', 30);

        return \in_array($range, self::ALLOWED_RANGES, true) ? $range : 30;
    }

    /**
     * BR-07-8: "growth trend = percentage change of the current week's
     * registrations against the previous week's." Null (not zero) when the
     * previous week had no registrations at all — a percentage change
     * against zero is undefined, not "infinite growth."
     */
    private function growthTrendPercent(
        \DateTimeImmutable $thisWeekFrom,
        \DateTimeImmutable $thisWeekTo,
        \DateTimeImmutable $lastWeekFrom,
        \DateTimeImmutable $lastWeekTo,
    ): ?float {
        $thisWeek = $this->accounts->countCreatedBetween($thisWeekFrom, $thisWeekTo);
        $lastWeek = $this->accounts->countCreatedBetween($lastWeekFrom, $lastWeekTo);

        if (0 === $lastWeek) {
            return null;
        }

        return round(100 * ($thisWeek - $lastWeek) / $lastWeek, 1);
    }

    /**
     * AC-07-2: "30-day user growth chart" — every day represented, zero
     * days filled in (the repository's own native query is sparse).
     *
     * @return list<array{date: string, count: int}>
     */
    private function growthChart(\DateTimeImmutable $now): array
    {
        $from = $now->modify(sprintf('-%d days', self::GROWTH_CHART_DAYS))->setTime(0, 0);
        $counts = $this->accounts->dailyRegistrationCounts($from, $now);

        $chart = [];
        for ($i = 0; $i < self::GROWTH_CHART_DAYS; ++$i) {
            $day = $from->modify(sprintf('+%d days', $i))->format('Y-m-d');
            $chart[] = ['date' => $day, 'count' => $counts[$day] ?? 0];
        }

        return $chart;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
