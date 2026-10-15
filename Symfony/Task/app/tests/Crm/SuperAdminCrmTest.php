<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerFlag;
use App\Scheduling\Entity\AttendanceRecord;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.12 — Super Admin Views System-Wide CRM.
 */
final class SuperAdminCrmTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-03-54: "CRM Master"/"All Players" lists every player from every
     * trainer, with tool-specific search by player name, trainer name, or
     * email.
     */
    public function testSuperAdminSeesAllPlayersAcrossEveryTrainerAndCanSearchByTrainerName(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerA);
        $playerA = $this->freshPlayerMembership($trainerA, 'CrmMasterA'.uniqid())->getPlayer();
        $this->activateTenant($trainerB);
        $playerB = $this->freshPlayerMembership($trainerB, 'CrmMasterB'.uniqid())->getPlayer();

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/crm/players?q='.$trainerA->getBusinessName());

        self::assertResponseIsSuccessful();
        $text = $crawler->filter('body')->text();
        self::assertStringContainsString($playerA->getFirstName(), $text, 'AC-03-54: search by trainer name finds that trainer\'s players.');
        self::assertStringNotContainsString($playerB->getFirstName(), $text);
    }

    /**
     * AC-03-55: filter by Trainer, skill level, age, gender, flags,
     * registration date range, last activity.
     */
    public function testSuperAdminFiltersPlayersByTrainerAndSkillLevel(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $membership = $this->freshPlayerMembership($trainerA, 'FilterTarget'.uniqid());
        $membership->setSkillLevel('Elite'.uniqid());
        $this->flush();

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf(
            '/super-admin/crm/players?trainer=%d&skillLevel=%s',
            $trainerA->getId(),
            $membership->getSkillLevel(),
        ));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $membership->getPlayer()->getFirstName());
    }

    /**
     * AC-03-56: clicking a player opens a Super-Admin-scoped detail; from
     * there Super Admin can apply flags across trainers.
     */
    public function testSuperAdminOpensAPlayerDetailAndAppliesAFlagAcrossTrainers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/crm/players/'.$membership->getId());
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Add flag')->form([
            'apply_flag[flagType]' => PlayerFlag::TYPE_ATTENDANCE_RISK,
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var \App\Crm\Repository\PlayerFlagRepository $flags */
        $flags = self::getContainer()->get(\App\Crm\Repository\PlayerFlagRepository::class);
        self::assertNotEmpty($flags->findActiveForPlayer($membership->getPlayer()), 'AC-03-56: Super Admin applied a flag.');
    }

    /**
     * AC-03-58: Super Admin can view and edit all player data and apply or
     * remove flags across trainers, but cannot edit trainer-specific
     * labels.
     */
    public function testSuperAdminCannotManageTrainerSpecificLabels(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $label = $this->createLabel($trainer, 'SuperAdminBlocked'.uniqid(), '#00B300');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        /** @var \Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(\Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(\App\Crm\Voter\LabelVoter::LABEL_MANAGE, $label),
            'AC-03-58: Super Admin cannot edit trainer-specific labels — no AdministrativeScope clause on LabelVoter at all.',
        );
    }

    /**
     * AC-03-57: system-wide Quick View — total players, sessions this week,
     * system-wide Top Players, system-wide flag counts, drill-down by
     * trainer.
     */
    public function testSystemWideDashboardShowsPlatformWideMetrics(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerA);
        $coachA = $this->coachMembershipFor($trainerA, $this->account('coach@practiceperfect.test'));
        $topPlayerA = $this->freshPlayerMembership($trainerA, 'SystemTopA'.uniqid())->getPlayer();
        $eventA = $this->createEvent($trainerA, ['title' => 'System Dashboard Event A '.uniqid()]);
        $rsvpA = $this->createRsvp($eventA, $topPlayerA);
        $this->createAttendanceRecord($eventA, $rsvpA, AttendanceRecord::STATUS_PRESENT, $coachA);
        $this->applyFlagToPlayer($trainerA, $topPlayerA, PlayerFlag::TYPE_SCHOLARSHIP, $trainerA->getOwnerAccount());

        $this->activateTenant($trainerB);
        $freshPlayerB = $this->freshPlayerMembership($trainerB, 'SystemWideB'.uniqid())->getPlayer();
        unset($freshPlayerB);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/crm/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'total players across all trainers');
        self::assertSelectorTextContains('body', 'sessions this week');
        self::assertStringContainsString($topPlayerA->getFirstName(), $crawler->filter('body')->text(), 'AC-03-57: system-wide Top Players includes players from any trainer.');
        self::assertSelectorTextContains('body', 'Scholarship');

        // Drill down by trainer.
        $crawler = $this->client->request('GET', '/super-admin/crm/dashboard?trainer='.$trainerA->getId());
        self::assertResponseIsSuccessful();
    }

    /**
     * AC-03-57 (deliberately not built): a system-wide revenue figure
     * requires Epic-05's `payment_record`, which does not exist in this
     * codebase — see SuperAdminCrmController's own docblock and the coder's
     * final report. The non-revenue half of this AC is fully covered by
     * testSystemWideDashboardShowsPlatformWideMetrics() above.
     */
    public function testSystemWideRevenueRequiresEpic05PaymentRecord(): void
    {
        self::markTestSkipped(
            'AC-03-57\'s revenue figure needs Epic-05\'s payment_record table, which does not exist yet '.
            'and is out of this epic\'s scope to build. Every other system-wide metric this AC names '.
            '(total players, sessions this week, Top Players, flag counts, trainer drill-down) is built '.
            'and covered by testSystemWideDashboardShowsPlatformWideMetrics().',
        );
    }
}
