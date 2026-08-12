<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Entity\PlayerFlag;
use App\Crm\Repository\PlayerFlagRepository;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.04 — Trainer Applies System Flags.
 */
final class PlayerFlagTest extends WebTestCase
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
     * AC-03-15/BR-03-6/7: one of the 8 system flags, optional note; applies
     * as a badge recording who and when.
     */
    public function testTrainerAppliesASystemFlagWithANote(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Add flag')->form([
            'apply_flag[flagType]' => PlayerFlag::TYPE_ATTENDANCE_RISK,
            'apply_flag[note]' => 'Missed the last three sessions.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Attendance risk');
        self::assertSelectorTextContains('body', 'Missed the last three sessions.');

        $this->activateTenant($trainer);
        /** @var PlayerFlagRepository $flags */
        $flags = self::getContainer()->get(PlayerFlagRepository::class);
        $active = $flags->findActiveForPlayer($membership->getPlayer());
        self::assertCount(1, $active, 'AC-03-15: the flag is recorded.');
        $flag = $active[0];
        self::assertSame(PlayerFlag::TYPE_ATTENDANCE_RISK, $flag->getFlagType());
        self::assertSame($trainer->getOwnerAccount()->getId(), $flag->getAppliedByAccount()->getId(), 'BR-03-7: who applied it is logged.');
        self::assertLessThanOrEqual(new \DateTimeImmutable(), $flag->getAppliedAt(), 'BR-03-7: when it was applied is logged.');
    }

    /**
     * AC-03-16: the player detail's full flag list, and a per-player flag
     * count.
     */
    public function testPlayerDetailShowsTheFullFlagListAndCount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_BEHAVIOR, $trainer->getOwnerAccount());
        $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_SCHOLARSHIP, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/players/'.$membership->getId());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '2 flags');
        self::assertSelectorTextContains('body', 'Behavior');
        self::assertSelectorTextContains('body', 'Scholarship');
    }

    /**
     * AC-03-17/BR-03-8: "Mark as resolved?" hides the flag from the active
     * view while keeping its history.
     */
    public function testResolvingAFlagHidesItFromTheActiveViewButKeepsHistory(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $flag = $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_INJURED, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        $form = $crawler->selectButton('Remove Flag')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PlayerFlagRepository $flags */
        $flags = self::getContainer()->get(PlayerFlagRepository::class);
        self::assertEmpty($flags->findActiveForPlayer($membership->getPlayer()), 'AC-03-17: hidden from the active view.');

        $all = $flags->findAllForPlayer($membership->getPlayer());
        self::assertCount(1, $all, 'BR-03-8: history preserved (the row still exists).');
        self::assertSame($flag->getId(), $all[0]->getId());
        self::assertSame(PlayerFlag::STATUS_RESOLVED, $all[0]->getStatus());
        self::assertNotNull($all[0]->getResolvedAt());
    }

    /**
     * BR-03-8: a trainer can reapply the same flag after resolution.
     */
    public function testTheSameFlagCanBeReappliedAfterResolution(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $flag = $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_CONTACT_PRIORITY, $trainer->getOwnerAccount());

        /** @var \App\Crm\Service\PlayerFlagService $flagService */
        $flagService = self::getContainer()->get(\App\Crm\Service\PlayerFlagService::class);
        $flagService->resolve($flag, $trainer->getOwnerAccount());

        // Reapplying now must succeed (no DuplicateActiveFlagException).
        $reapplied = $flagService->apply($membership, PlayerFlag::TYPE_CONTACT_PRIORITY, $trainer->getOwnerAccount(), null);

        self::assertNotSame($flag->getId(), $reapplied->getId(), 'BR-03-8: reapplication is a fresh row, not an update.');
        self::assertTrue($reapplied->isActive());
    }

    /**
     * BR-03-8 (negative): a SECOND active instance of the same flag type on
     * the same player is refused while the first is still active.
     */
    public function testApplyingTheSameActiveFlagTwiceIsRefused(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $this->applyFlagToPlayer($trainer, $membership->getPlayer(), PlayerFlag::TYPE_HIGH_NO_SHOW_RATE, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        $form = $crawler->selectButton('Add flag')->form([
            'apply_flag[flagType]' => PlayerFlag::TYPE_HIGH_NO_SHOW_RATE,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'already has an active');

        $this->activateTenant($trainer);
        /** @var PlayerFlagRepository $flags */
        $flags = self::getContainer()->get(PlayerFlagRepository::class);
        self::assertCount(1, $flags->findActiveForPlayer($membership->getPlayer()), 'BR-03-8: at most one active instance.');
    }

    /**
     * AC-03-18, AC-03-56, AC-03-58: Super Admin can apply/view/remove flags
     * across trainers, gated by `AdministrativeScope` opened for the SPECIFIC
     * trainer. Checked directly against the voter, the same
     * `AuthorizationCheckerInterface` pattern
     * `Administration\ImpersonationTest` already establishes for a
     * hidden-by-template, voter-enforced capability (no `crm_super_admin_*`
     * write route is drawn upstream — see the coder's final report).
     */
    public function testSuperAdminCanManageFlagsAcrossTrainersOnlyWhileAdministrativeScopeIsOpenForThatTrainer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $otherTrainer = $this->trainer('baseline-athletics');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);

        $superAdmin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($superAdmin);
        /** @var \Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(\Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(\App\Crm\Voter\PlayerVoter::PLAYER_FLAG_MANAGE, $membership),
            'No AdministrativeScope open yet: bare Super Admin role is insufficient.',
        );

        /** @var \App\Platform\Tenancy\AdministrativeScope $scope */
        $scope = self::getContainer()->get(\App\Platform\Tenancy\AdministrativeScope::class);
        $scope->openFor($otherTrainer, $superAdmin);
        self::assertFalse(
            $authChecker->isGranted(\App\Crm\Voter\PlayerVoter::PLAYER_FLAG_MANAGE, $membership),
            'AC-03-56: the scope must be open for THIS SPECIFIC trainer, not merely some trainer.',
        );

        $scope->openFor($trainer, $superAdmin);
        self::assertTrue(
            $authChecker->isGranted(\App\Crm\Voter\PlayerVoter::PLAYER_FLAG_MANAGE, $membership),
            'AC-03-18/AC-03-58: Super Admin can apply/remove flags once the scope is open for this trainer.',
        );
    }
}
