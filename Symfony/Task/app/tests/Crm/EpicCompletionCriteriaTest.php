<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\MembershipService;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * The epic-level criteria this suite has not already covered as a side
 * effect of another AC, plus the criteria that genuinely cannot be
 * machine-verified — each skipped here with an honest, specific reason
 * rather than faked. See the coder's final report for the same list with
 * full context.
 */
final class EpicCompletionCriteriaTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    /**
     * AC-03-68: every player-trainer association records its source
     * (ShareLink, event registration, or coach invitation) and, if via
     * ShareLink, which one. Already fully implemented by Epic-01's
     * `MembershipService` — `player_trainer_membership.source` and
     * `.share_link_id` (BR-01-14, database-designer-schema.md
     * "`player_trainer_membership`") — including the fourth source,
     * `camp_registration` (owner decision A3), which predates this epic.
     * This test asserts the behavior directly rather than adding a second
     * write path, per the task brief.
     */
    public function testEveryPlayerTrainerAssociationRecordsItsSourceAndShareLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        /** @var MembershipService $memberships */
        $memberships = self::getContainer()->get(MembershipService::class);
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        // Source 1: ShareLink — records which link.
        /** @var \App\Identity\Repository\ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(\App\Identity\Repository\ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink);
        $viaShareLink = new PlayerProfile('ViaShareLinkSource'.uniqid(), new \DateTimeImmutable('-15 years'));
        $em->persist($viaShareLink);
        $em->flush();
        $m1 = $memberships->associatePlayer($trainer, $viaShareLink, PlayerTrainerMembership::SOURCE_SHARELINK, $staticLink);
        self::assertSame(PlayerTrainerMembership::SOURCE_SHARELINK, $m1->getSource());
        self::assertSame($staticLink->getId(), $m1->getShareLink()?->getId(), 'AC-03-68: which ShareLink is recorded.');

        // Source 2: event registration — no ShareLink.
        $viaEvent = new PlayerProfile('ViaEventSource'.uniqid(), new \DateTimeImmutable('-15 years'));
        $em->persist($viaEvent);
        $em->flush();
        $m2 = $memberships->associatePlayer($trainer, $viaEvent, PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION);
        self::assertSame(PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION, $m2->getSource());
        self::assertNull($m2->getShareLink());

        // Source 3: coach invitation — no ShareLink recorded on the
        // membership itself (the coach's OWN ShareLink invite is a
        // separate, Epic-03 concept — AC-03-50..53 — from this source enum
        // value, which predates it in Epic-01).
        $viaCoach = new PlayerProfile('ViaCoachSource'.uniqid(), new \DateTimeImmutable('-15 years'));
        $em->persist($viaCoach);
        $em->flush();
        $m3 = $memberships->associatePlayer($trainer, $viaCoach, PlayerTrainerMembership::SOURCE_COACH_INVITE);
        self::assertSame(PlayerTrainerMembership::SOURCE_COACH_INVITE, $m3->getSource());

        // Source 4 (owner decision A3, predates this epic): camp registration.
        $viaCamp = new PlayerProfile('ViaCampSource'.uniqid(), new \DateTimeImmutable('-15 years'));
        $em->persist($viaCamp);
        $em->flush();
        $m4 = $memberships->associatePlayer($trainer, $viaCamp, PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION);
        self::assertSame(PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION, $m4->getSource());

        // Confirm all four persisted correctly (not just in-memory).
        /** @var PlayerTrainerMembershipRepository $repo */
        $repo = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        foreach ([$m1, $m2, $m3, $m4] as $m) {
            $reloaded = $repo->find($m->getId());
            self::assertNotNull($reloaded);
            self::assertSame($m->getSource(), $reloaded->getSource());
        }
    }

    /**
     * AC-03-7: "The player list loads in under 2 seconds for 500 players."
     * A load/performance target, not a functional-test assertion — a
     * PHPUnit wall-clock timing on a single request in a shared,
     * unthrottled test container would not be a meaningful measurement
     * either way, matching Epic-02's own EpicCompletionCriteriaTest
     * precedent for AC-02-69.
     */
    public function testPlayerListPerformanceTarget(): void
    {
        self::markTestSkipped(
            'AC-03-7\'s "under 2 seconds for 500 players" is a load/performance target requiring dedicated '.
            'load-testing tooling against a realistically sized dataset, not a PHPUnit functional-test '.
            'assertion. PlayerSegmentationRepository::search() itself is proven correct (filters, AND '.
            'logic, tenancy) by PlayerSegmentationTest; only its throughput at 500-row scale is unverifiable here.',
        );
    }

    /**
     * AC-03-42: "The Quick View dashboard loads in under 2 seconds." Same
     * class of performance target as AC-03-7/69.
     */
    public function testQuickViewDashboardPerformanceTarget(): void
    {
        self::markTestSkipped(
            'AC-03-42\'s "under 2 seconds" is a load/performance target requiring dedicated load-testing '.
            'tooling, not a PHPUnit functional-test assertion. QuickViewDashboardService::build() itself '.
            'is proven correct by QuickViewDashboardTest; only its wall-clock latency is unverifiable here.',
        );
    }

    /**
     * AC-03-69: "Search results return in under 500ms." Same class of
     * performance target as AC-03-7/42.
     */
    public function testSearchResponseTimePerformanceTarget(): void
    {
        self::markTestSkipped(
            'AC-03-69\'s "under 500ms" is a load/performance target requiring dedicated load-testing '.
            'tooling against a realistic dataset and production-like infrastructure, not a PHPUnit '.
            'functional-test assertion — a single request\'s wall-clock time in a shared, unthrottled '.
            'test container measures the container, not the query.',
        );
    }

    /**
     * AC-03-70 (the epic's own text: "Process gate, not product behavior"):
     * "Epic-03 is considered complete only once the demo is approved, all
     * P0 and P1 open questions are resolved, the label and flag systems are
     * validated, and the Top Players algorithm is confirmed." A human
     * sign-off/process gate, not something a test can assert.
     */
    public function testEpicApprovalIsAProcessGateNotProductBehavior(): void
    {
        self::markTestSkipped(
            'AC-03-70 is explicitly a process gate ("not product behavior" per the epic\'s own text) — '.
            'demo approval and open-question resolution are human sign-offs, not something a test can '.
            'assert. The Top Players algorithm itself (BR-03-16..19) is fully verified by '.
            'QuickViewDashboardTest::testTopPlayersRankingFollowsTheConfirmedAlgorithm().',
        );
    }
}
