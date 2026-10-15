<?php

declare(strict_types=1);

namespace App\Content\Controller;

use App\Identity\Entity\Account;
use App\Platform\Service\CrossTenantReadService;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-04.11: Super Admin's system-wide LPPP analytics dashboard.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md AC-04-37..40
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class SuperAdminContentController extends AbstractController
{
    public function __construct(
        private readonly CrossTenantReadService $crossTenantReads,
    ) {
    }

    /**
     * AC-04-37..40: content stats, engagement stats, growth trends, and an
     * optional per-trainer drill-down.
     */
    #[Route('/super-admin/content/analytics', name: 'content_super_admin_analytics', methods: ['GET'])]
    public function analytics(Request $request): Response
    {
        $actor = $this->actor();
        $now = new \DateTimeImmutable();
        $weekStart = $now->modify('monday this week')->setTime(0, 0);

        $trainerIdParam = $request->query->get('trainer');
        $trainerId = null === $trainerIdParam || '' === $trainerIdParam ? null : (int) $trainerIdParam;

        return $this->render('content/super_admin_analytics.html.twig', [
            'contentStats' => $this->crossTenantReads->contentStats($actor),
            'topDrills' => $this->crossTenantReads->topPublicDrills($actor),
            'topCreators' => $this->crossTenantReads->topContentCreators($actor),
            'engagement' => $this->crossTenantReads->contentEngagementStats($actor, $weekStart),
            'trends' => $this->crossTenantReads->contentGrowthTrends($actor, $now),
            'trainerId' => $trainerId,
            'trainerDrilldown' => null === $trainerId ? null : $this->crossTenantReads->trainerContentStats($actor, $trainerId),
        ]);
    }

    /**
     * AC-04-40: CSV export.
     */
    #[Route('/super-admin/content/analytics/export', name: 'content_super_admin_analytics_export', methods: ['GET'])]
    public function export(): StreamedResponse
    {
        $actor = $this->actor();
        $now = new \DateTimeImmutable();

        $response = new StreamedResponse(function () use ($actor, $now): void {
            $out = fopen('php://output', 'w');
            \assert(false !== $out);

            fputcsv($out, ['Metric', 'Value'], escape: '\\');
            $stats = $this->crossTenantReads->contentStats($actor);
            foreach ($stats as $key => $value) {
                fputcsv($out, [$key, (string) $value], escape: '\\');
            }

            fputcsv($out, [], escape: '\\');
            fputcsv($out, ['Week Start', 'Content Created', 'Views', 'New Public Items'], escape: '\\');
            foreach ($this->crossTenantReads->contentGrowthTrends($actor, $now) as $row) {
                fputcsv($out, [$row['weekStart'], $row['contentCreated'], $row['views'], $row['newPublicItems']], escape: '\\');
            }

            fclose($out);
        });

        $response->headers->set('Content-Type', 'text/csv');
        $response->headers->set('Content-Disposition', 'attachment; filename="lppp-analytics.csv"');

        return $response;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
