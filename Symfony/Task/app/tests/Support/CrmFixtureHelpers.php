<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Crm\Entity\Label;
use App\Crm\Entity\PlayerFlag;
use App\Crm\Entity\PlayerLabel;
use App\Crm\Entity\PlayerNote;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Entity\Trainer;
use App\Scheduling\Entity\Event;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Crm-specific fixture helpers, built on FixtureHelpers (AppFixtures
 * lookups, tenant activation) — the same "set up via the entity layer,
 * exercise the feature under test via HTTP" split
 * SchedulingFixtureHelpers already establishes.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`.
 */
trait CrmFixtureHelpers
{
    protected function createLabel(Trainer $trainer, string $name, string $colorHex = '#00B300'): Label
    {
        $label = new Label($trainer, $name, $colorHex);
        $this->crmEntityManager()->persist($label);
        $this->crmEntityManager()->flush();

        return $label;
    }

    protected function applyLabelToPlayer(Trainer $trainer, PlayerProfile $player, Label $label, Account $appliedBy): PlayerLabel
    {
        $playerLabel = new PlayerLabel($trainer, $player, $label, $appliedBy);
        $this->crmEntityManager()->persist($playerLabel);
        $this->crmEntityManager()->flush();

        return $playerLabel;
    }

    protected function applyFlagToPlayer(
        Trainer $trainer,
        PlayerProfile $player,
        string $flagType,
        Account $appliedBy,
        ?string $note = null,
    ): PlayerFlag {
        $flag = new PlayerFlag($trainer, $player, $flagType, $appliedBy, $note);
        $this->crmEntityManager()->persist($flag);
        $this->crmEntityManager()->flush();

        return $flag;
    }

    protected function createPlayerNote(
        Trainer $trainer,
        PlayerProfile $player,
        string $noteType,
        string $text,
        Account $author,
        ?Event $event = null,
    ): PlayerNote {
        $note = new PlayerNote($trainer, $player, $noteType, $text, $author, $event);
        $this->crmEntityManager()->persist($note);
        $this->crmEntityManager()->flush();

        return $note;
    }

    /**
     * A brand-new player, freshly associated with $trainer — never reuse the
     * shared fixture players (Pat, Alex, Blake) for a test that counts exact
     * flags/labels/notes or selects a single button/row by label text: other
     * tests in a whole-suite run apply their own labels/flags/notes to those
     * same shared players and nothing resets between test methods (the same
     * isolation problem SchedulingFixtureHelpers::createCoach() already
     * solves for coaches — this is that same shape for players).
     */
    protected function freshPlayerMembership(Trainer $trainer, string $firstNamePrefix = 'Test Player'): PlayerTrainerMembership
    {
        $player = new PlayerProfile($firstNamePrefix.' '.uniqid(), new \DateTimeImmutable('-15 years'));
        $this->crmEntityManager()->persist($player);
        $this->crmEntityManager()->flush();

        /** @var \App\Identity\Service\MembershipService $membershipService */
        $membershipService = self::getContainer()->get(\App\Identity\Service\MembershipService::class);

        return $membershipService->associatePlayer($trainer, $player, PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION);
    }

    /**
     * Every Crm route is keyed on {membership}, not {player} — this is the
     * lookup every Crm test needs to turn a fixture player into the id its
     * routes expect. Requires the tenant to already be active.
     */
    protected function membershipFor(Trainer $trainer, PlayerProfile $player): PlayerTrainerMembership
    {
        /** @var PlayerTrainerMembershipRepository $repository */
        $repository = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $repository->findOneByTrainerAndPlayer($trainer, $player);

        self::assertNotNull($membership, 'Expected an existing PlayerTrainerMembership for this trainer/player pair.');

        return $membership;
    }

    /**
     * Flushes pending changes — for tests that mutate an already-persisted
     * entity in place (e.g. `$membership->setSkillLevel(...)`) rather than
     * going through one of the create* helpers above, which each flush
     * themselves.
     */
    protected function flush(): void
    {
        $this->crmEntityManager()->flush();
    }

    private function crmEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
