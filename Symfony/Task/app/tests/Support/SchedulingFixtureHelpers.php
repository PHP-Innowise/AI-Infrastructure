<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\MembershipService;
use App\Platform\Entity\AccountTrainerLink;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AccountTrainerLinkRepository;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\EventInvitation;
use App\Scheduling\Entity\Rsvp;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;

/**
 * Scheduling-specific fixture helpers, built on top of the base
 * FixtureHelpers trait (AppFixtures lookups, tenant activation). Builds test
 * data directly through entity constructors + persist/flush, under an
 * already-activated tenant, rather than through the full HTTP form flow —
 * the same "set up via the service/entity layer, exercise the feature under
 * test via HTTP" split ChildApprovalTest already establishes.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`.
 */
trait SchedulingFixtureHelpers
{
    /**
     * @param array{
     *   title?: string, eventType?: string, startsAt?: \DateTimeImmutable, endsAt?: \DateTimeImmutable,
     *   location?: string, capacity?: int, visibility?: string, description?: ?string,
     *   minAge?: ?int, maxAge?: ?int, skillLevels?: ?list<string>, genders?: ?list<string>,
     *   usdPricingEnabled?: bool, usdPriceMinorUnits?: int, tokenPricingEnabled?: bool, tokenPrice?: int,
     * } $overrides
     */
    protected function createEvent(Trainer $trainer, array $overrides = []): Event
    {
        $now = new \DateTimeImmutable();

        $event = new Event(
            $trainer,
            $overrides['title'] ?? 'Test Event '.uniqid(),
            $overrides['eventType'] ?? Event::TYPE_TRAINING_SESSION,
            $overrides['startsAt'] ?? $now->modify('+2 days'),
            $overrides['endsAt'] ?? ($overrides['startsAt'] ?? $now->modify('+2 days'))->modify('+1 hour'),
            $overrides['location'] ?? 'Test Court',
            $overrides['capacity'] ?? 10,
            $overrides['visibility'] ?? Event::VISIBILITY_PUBLIC,
            $overrides['description'] ?? null,
            $overrides['minAge'] ?? null,
            $overrides['maxAge'] ?? null,
            $overrides['skillLevels'] ?? null,
            $overrides['genders'] ?? null,
        );
        $event->setUsdPricing($overrides['usdPricingEnabled'] ?? false, $overrides['usdPriceMinorUnits'] ?? 0);
        $event->setTokenPricing($overrides['tokenPricingEnabled'] ?? false, $overrides['tokenPrice'] ?? 1);

        $this->entityManager()->persist($event);
        $this->entityManager()->flush();

        return $event;
    }

    protected function createRsvp(
        Event $event,
        PlayerProfile $player,
        string $paymentMethod = Rsvp::METHOD_FREE,
        string $status = Rsvp::STATUS_CONFIRMED,
    ): Rsvp {
        $rsvp = new Rsvp($event->getTrainer(), $event, $player, $paymentMethod, new \DateTimeImmutable());

        if (Rsvp::STATUS_PENDING_PARENT_APPROVAL === $status) {
            $rsvp->markPendingParentApproval();
        } elseif (Rsvp::STATUS_CONFIRMED === $status && Rsvp::METHOD_FREE !== $paymentMethod) {
            $rsvp->confirm(new \DateTimeImmutable());
        } elseif (Rsvp::STATUS_CANCELED === $status) {
            $rsvp->cancel('Test setup.', new \DateTimeImmutable());
        }

        $this->entityManager()->persist($rsvp);
        $this->entityManager()->flush();

        return $rsvp;
    }

    protected function createCoachAssignment(Event $event, CoachMembership $coach, string $status = CoachAssignment::STATUS_CONFIRMED): CoachAssignment
    {
        $assignment = new CoachAssignment($event->getTrainer(), $event, $coach, new \DateTimeImmutable());

        if (CoachAssignment::STATUS_CONFIRMED === $status) {
            $assignment->confirm(new \DateTimeImmutable());
        } elseif (CoachAssignment::STATUS_DECLINED === $status) {
            $assignment->decline('Test setup.');
        }

        $this->entityManager()->persist($assignment);
        $this->entityManager()->flush();

        return $assignment;
    }

    protected function createInvitation(Event $event, PlayerProfile $player, Account $invitedBy): EventInvitation
    {
        $invitation = new EventInvitation($event->getTrainer(), $event, $player, $invitedBy);
        $this->entityManager()->persist($invitation);
        $this->entityManager()->flush();

        return $invitation;
    }

    protected function createAttendanceRecord(Event $event, Rsvp $rsvp, string $status, CoachMembership $coach, ?\DateTimeImmutable $recordedAt = null): AttendanceRecord
    {
        $record = new AttendanceRecord($event->getTrainer(), $event, $rsvp->getPlayer(), $rsvp, $status, $coach, $recordedAt ?? new \DateTimeImmutable());
        $this->entityManager()->persist($record);
        $this->entityManager()->flush();

        return $record;
    }

    /**
     * A fresh, dedicated coach account + active CoachMembership under
     * $trainer — never reuse the shared fixture coach ("Casey Coach",
     * behind coach@practiceperfect.test) for a test needing an
     * uncontaminated coach: other tests elsewhere in a full-suite run set
     * that fixture coach's own availability, assignments, and attendance
     * history, exactly the isolation problem CoachAssignmentTest's own
     * (private, undupllicated-here) createSecondCoach() already solves —
     * this is that same shape, shared for every other Scheduling test file
     * that also needs it. Also creates $trainer's own AccountTrainerLink —
     * see giveChildOwnLogin()'s own docblock for exactly why a brand-new
     * account needs one before TenantResolver's single-tenant fallback can
     * resolve any tenant context for its own logged-in HTTP requests at
     * all (a CoachMembership row alone is not what TenantResolver reads).
     */
    protected function createCoach(Trainer $trainer, string $email): Account
    {
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);
        $account = new Account($email, '', AccountRole::Coach);
        $account->changePasswordHash($hasher->hashPassword($account, 'password'));
        $account->verifyEmail();

        $this->entityManager()->persist($account);
        $this->entityManager()->persist(new AccountProfile($account, 'Test', 'Coach'));
        $this->entityManager()->flush();

        $this->activateTenant($trainer);
        $this->entityManager()->persist(new CoachMembership($trainer, $account, CoachMembership::STATUS_ACTIVE));

        /** @var AccountTrainerLinkRepository $links */
        $links = self::getContainer()->get(AccountTrainerLinkRepository::class);
        $links->add(new AccountTrainerLink($account, $trainer, AccountRole::Coach->value));

        $this->entityManager()->flush();

        return $account;
    }

    protected function coachMembershipFor(Trainer $trainer, Account $coachAccount): CoachMembership
    {
        $membership = $this->coachMemberships()->findOneForAccountInActiveTenant($coachAccount);

        self::assertNotNull($membership, 'Fixture coach membership is missing.');

        return $membership;
    }

    /**
     * Explicitly selects $trainer as the logged-in player's active tenant
     * context, via a real request to platform_context_trainer_switch —
     * never assume "the player's only trainer resolves automatically":
     * several tests across this suite (Epic-01's own TenancyIsolationTest
     * and others, plus this suite's own multi-trainer tests) associate the
     * shared fixture player "Pat" with more than one trainer, and once that
     * has happened anywhere in a given test run, TenantResolver's
     * single-tenant fallback (specs/architect-architecture.md "Layer 3")
     * no longer applies for the rest of the run — there is no per-test
     * database reset in this suite. $client must already be logged in as
     * the player.
     */
    protected function switchPlayerToTrainer(KernelBrowser $client, Trainer $trainer): void
    {
        $crawler = $client->request('GET', '/dashboard');
        $forms = $crawler->filter('form[action*="/context/trainer/'.$trainer->getId().'"]');

        // Already the sole/active context (the switch form for it renders
        // only when the player has more than one trainer) — nothing to do.
        if (0 === $forms->count()) {
            return;
        }

        $client->submit($forms->form());
    }

    /**
     * Switches the logged-in parent's current player context to one of
     * their children, via FamilyController's own switch form — the same
     * mechanism PrivateEventTest establishes inline. $client must already
     * be logged in as the parent (and, if the parent trains with more than
     * one trainer, already have called switchPlayerToTrainer() for the
     * intended one first).
     */
    protected function switchToChild(KernelBrowser $client, string $firstName): void
    {
        $crawler = $client->request('GET', '/portal/family');
        $form = $crawler->selectButton('Switch to '.$firstName)->form();
        $client->submit($form);
    }

    /**
     * Ensures $player is actively associated with $trainer, reactivating if
     * needed — never assume a fixture child's own original association
     * survives the whole suite run: some Epic-01 tests (e.g.
     * ChildTrainerAssociationTest) remove the shared fixture child "Alex"
     * from 'peak-performance' as part of their own scenario, and that
     * removal persists for the rest of the run (no per-test database
     * reset). MembershipService::associatePlayer() is the sole creator of
     * PlayerTrainerMembership rows and is idempotent — a no-op when already
     * active.
     */
    protected function ensureActivePlayerMembership(Trainer $trainer, PlayerProfile $player): void
    {
        /** @var MembershipService $memberships */
        $memberships = self::getContainer()->get(MembershipService::class);
        $memberships->associatePlayer($trainer, $player, PlayerTrainerMembership::SOURCE_COACH_INVITE);
    }

    /**
     * AC-01-20/AC-02-23/33: gives a child profile its own separate login,
     * distinct from the parent's — needed to exercise
     * ChildApprovalVoter::voteBypass()'s "actor is the child's own login"
     * branch. A parent's own context-switch (switchPlayerToTrainer() +
     * FamilyController's child-switch) can never reach that branch: the
     * parent-owns-the-link check short-circuits true first, per that
     * voter's own docblock ("the parent's own action already IS the
     * approval"). loginUser() bypasses password verification entirely, so
     * the hash here is never actually checked against anything.
     *
     * Also creates $trainer's own AccountTrainerLink for the new account,
     * mirroring MembershipService::syncAccountTrainerLink() — without it,
     * TenantResolver::resolve() (Source 4/"single-tenant fallback") finds
     * no active link for this brand-new account at all and the request's
     * tenant context stays unresolved, which would make every
     * trainer-scoped read (RLS + the Doctrine filter) come back empty
     * regardless of $child's own, separate PlayerTrainerMembership.
     * ChildProfileService never builds this combination itself (children
     * it creates start with no self-account at all — see its own
     * addChildToExistingTrainer()), so no existing service call covers
     * this shape end-to-end.
     *
     * Idempotent per child: PlayerProfile::attachSelfAccount() throws if a
     * DIFFERENT account is already attached, and the shared fixture
     * players (Alex, Blake) persist across every test method in a whole
     * suite run (no per-test database reset) — a second test method
     * reusing the same child would otherwise hit that guard. If $child
     * already has a self-account, this returns the existing one rather
     * than attempting to attach $email as a second one.
     */
    protected function giveChildOwnLogin(PlayerProfile $child, string $email, Trainer $trainer): Account
    {
        $existing = $child->getSelfAccount();

        if (null !== $existing) {
            return $existing;
        }

        $account = new Account($email, 'not-a-real-hash', AccountRole::Player);
        $this->entityManager()->persist($account);
        $child->attachSelfAccount($account);

        /** @var AccountTrainerLinkRepository $links */
        $links = self::getContainer()->get(AccountTrainerLinkRepository::class);
        $links->add(new AccountTrainerLink($account, $trainer, AccountRole::Player->value));

        $this->entityManager()->flush();

        return $account;
    }

    /**
     * The fixture player "Alex" — a child of player@practiceperfect.test,
     * associated with the 'peak-performance' trainer.
     */
    protected function alexPlayer(): PlayerProfile
    {
        return $this->childPlayerByFirstName('Alex');
    }

    /**
     * The fixture player "Blake" — a child of player@practiceperfect.test,
     * associated with the 'baseline-athletics' trainer (a different tenant
     * than Alex — useful for cross-tenant tests).
     */
    protected function blakePlayer(): PlayerProfile
    {
        return $this->childPlayerByFirstName('Blake');
    }

    /**
     * The fixture player "Pat" — the adult self-training player, also the
     * parent of Alex and Blake.
     */
    protected function patPlayer(): PlayerProfile
    {
        /** @var PlayerProfileRepository $repository */
        $repository = self::getContainer()->get(PlayerProfileRepository::class);
        $player = $repository->findOneForSelfAccount($this->account('player@practiceperfect.test'));

        self::assertNotNull($player, 'Fixture player "Pat" is missing.');

        return $player;
    }

    private function childPlayerByFirstName(string $firstName): PlayerProfile
    {
        /** @var ParentChildLinkRepository $links */
        $links = self::getContainer()->get(ParentChildLinkRepository::class);
        $parent = $this->account('player@practiceperfect.test');

        $link = current(array_filter(
            $links->findByParent($parent),
            static fn ($l) => $firstName === $l->getChildPlayer()->getFirstName(),
        ));

        self::assertNotFalse($link, sprintf('Fixture child "%s" is missing.', $firstName));

        return $link->getChildPlayer();
    }

    private function coachMemberships(): CoachMembershipRepository
    {
        /** @var CoachMembershipRepository $repository */
        $repository = self::getContainer()->get(CoachMembershipRepository::class);

        return $repository;
    }

    private function entityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
