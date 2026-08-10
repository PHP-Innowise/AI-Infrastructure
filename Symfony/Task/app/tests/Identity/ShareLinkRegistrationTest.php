<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Tenancy\TenantContext;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-01.02 — Player Registers via ShareLink.
 */
final class ShareLinkRegistrationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-9: clicking a trainer's ShareLink while logged out shows the
     * registration/login prompt.
     */
    public function testLoggedOutVisitorSeesTheRegistrationPrompt(): void
    {
        $this->client->request('GET', '/join/join-peak-performance');

        self::assertResponseIsSuccessful();
        self::assertSelectorExists('form[name="player_registration"]');
    }

    /**
     * AC-01-9: clicking it while already logged in redirects straight to the
     * association step (an "instant association" for a single-trainer actor).
     */
    public function testLoggedInVisitorIsRedirectedToAssociation(): void
    {
        // A fresh player with no trainer relationships at all yet, so
        // acceptance is unambiguous.
        $newPlayer = $this->registerFreshPlayer('instant-assoc@example.test');
        $this->client->loginUser($newPlayer);

        $this->client->request('GET', '/join/join-baseline-athletics');

        self::assertResponseRedirects('/join/join-baseline-athletics/associate');
    }

    /**
     * AC-01-10: the form captures name, email, password, parent phone, and
     * player name/age/gender. AC-01-11: auto-associated with the trainer, a
     * player profile exists in that trainer's CRM.
     */
    public function testRegisteringCapturesAllFieldsAndAutoAssociatesWithTheTrainer(): void
    {
        $crawler = $this->client->request('GET', '/join/join-peak-performance');
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Rory',
            'player_registration[accountLastName]' => 'Registrant',
            'player_registration[email]' => 'rory.registrant@example.test',
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[parentPhone]' => '555-0101',
            'player_registration[playerFirstName]' => 'Rory',
            'player_registration[playerDateOfBirth]' => sprintf('%d-06-15', ((int) date('Y')) - 30),
            'player_registration[playerGender]' => 'unspecified',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $account = $this->account('rory.registrant@example.test');
        self::assertSame('555-0101', $account->getProfile()?->getPhone(), 'AC-01-10: parent phone captured.');

        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $player = $playerProfiles->findOneForSelfAccount($account);
        self::assertNotNull($player, 'AC-01-11: a player profile exists for the new account.');
        self::assertSame('Rory', $player->getFirstName());

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($this->trainer('peak-performance'), $player);
        self::assertNotNull($membership, 'AC-01-11: the player is associated with the trainer who sent the link.');
        self::assertTrue($membership->isActive());
        self::assertSame(PlayerTrainerMembership::SOURCE_SHARELINK, $membership->getSource());
    }

    /**
     * AC-01-12: a confirmation email is sent after registration.
     */
    public function testRegistrationSendsAConfirmationEmail(): void
    {
        $crawler = $this->client->request('GET', '/join/join-peak-performance');
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Coral',
            'player_registration[accountLastName]' => 'Confirmed',
            'player_registration[email]' => 'coral.confirmed@example.test',
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Coral',
            'player_registration[playerDateOfBirth]' => sprintf('%d-03-03', ((int) date('Y')) - 25),
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        // Two emails queue on registration: BR-01-5's verification email and
        // AC-01-12's confirmation — both addressed to the new registrant.
        self::assertQueuedEmailCount(2);
        $addressedToNewAccount = array_filter(
            self::getMailerMessages(),
            static fn ($message) => $message instanceof Email && \in_array('coral.confirmed@example.test', array_map(
                static fn ($address) => $address->getAddress(),
                $message->getTo(),
            ), true),
        );
        self::assertCount(2, $addressedToNewAccount, 'AC-01-12: a confirmation email is sent to the new registrant.');
    }

    /**
     * AC-01-13: an existing account accepting a DIFFERENT trainer's link is
     * associated with that second trainer, without a duplicate account.
     */
    public function testExistingAccountAcceptingASecondTrainersLinkIsAssociatedNotDuplicated(): void
    {
        $existing = $this->account('player@practiceperfect.test');
        $accountId = $existing->getId();

        $this->client->loginUser($existing);
        $crawler = $this->client->request('GET', '/join/join-baseline-athletics/associate');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Continue')->form([
            'family_member_selection[includeSelf]' => true,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        // Same account id: no duplicate was created.
        $stillSame = $this->account('player@practiceperfect.test');
        self::assertSame($accountId, $stillSame->getId());

        $this->activateTenant($this->trainer('baseline-athletics'));
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($stillSame);
        self::assertNotNull($selfPlayer);

        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($this->trainer('baseline-athletics'), $selfPlayer);
        self::assertNotNull($membership, 'AC-01-13: associated with the second trainer.');
        self::assertTrue($membership->isActive());
    }

    /**
     * AC-01-14: a parent with children sees a selection prompt listing "Me"
     * and every child; only the selected members are associated.
     */
    public function testParentWithChildrenSeesAFamilySelectionPrompt(): void
    {
        $parent = $this->account('player@practiceperfect.test');
        $this->client->loginUser($parent);

        $crawler = $this->client->request('GET', '/join/join-baseline-athletics/associate');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Who will train with Baseline Athletics');
        self::assertSelectorExists('input[type="checkbox"][id^="family_member_selection_children"]');

        // Select only "Me", not the children.
        $form = $crawler->selectButton('Continue')->form([
            'family_member_selection[includeSelf]' => true,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('baseline-athletics'));
        /** @var ParentChildLinkRepository $parentChildLinks */
        $parentChildLinks = self::getContainer()->get(ParentChildLinkRepository::class);
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);

        // Alex (fixtures: associated only with peak-performance) was never
        // selected here, so must stay unassociated with Baseline. Blake is
        // deliberately not checked: the fixtures already associate Blake
        // with Baseline independently of this flow, which would make that
        // assertion meaningless either way.
        $alexLink = current(array_filter(
            $parentChildLinks->findByParent($parent),
            static fn ($link) => 'Alex' === $link->getChildPlayer()->getFirstName(),
        ));
        self::assertNotFalse($alexLink, 'Fixtures should provide Alex as one of Pat\'s children.');

        $membership = $memberships->findOneByTrainerAndPlayer($this->trainer('baseline-athletics'), $alexLink->getChildPlayer());
        self::assertTrue(
            null === $membership || !$membership->isActive(),
            'AC-01-14: an unselected child must not be associated with the new trainer.',
        );
    }

    /**
     * AC-01-15: multi-trainer players see separated, isolated contexts — a
     * membership under one trainer is invisible while a different trainer's
     * tenant is active, and a context switcher is available.
     */
    public function testMultiTrainerContextsAreSeparatedAndSwitchable(): void
    {
        $player = $this->account('player@practiceperfect.test');

        // Associate the player's own profile with Baseline too, so they now
        // have two active trainer contexts.
        $this->client->loginUser($player);
        $crawler = $this->client->request('GET', '/join/join-baseline-athletics/associate');
        $form = $crawler->selectButton('Continue')->form(['family_member_selection[includeSelf]' => true]);
        $this->client->submit($form);

        $this->client->request('GET', '/dashboard');
        self::assertResponseIsSuccessful();
        // AC-01-15: a context switcher is available in navigation once more
        // than one trainer relationship exists.
        self::assertSelectorTextContains('body', 'Switch to this trainer');

        /** @var AccountTrainerLinkRepository $links */
        $links = self::getContainer()->get(AccountTrainerLinkRepository::class);
        self::assertGreaterThanOrEqual(2, \count($links->findActiveFor($player)), 'The player now has two active trainer relationships.');

        // Isolation: activating one tenant must not surface the other's rows.
        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($player);
        self::assertNotNull($selfPlayer);

        $this->activateTenant($this->trainer('peak-performance'));
        $underPeak = $memberships->findActiveForPlayer($selfPlayer);
        foreach ($underPeak as $m) {
            self::assertSame('peak-performance', $m->getTrainer()->getSlug(), 'AC-01-15: no combined cross-trainer view.');
        }
    }

    private function registerFreshPlayer(string $email): \App\Identity\Entity\Account
    {
        // Registering auto-logs the new account in (see ShareLinkController::register());
        // the caller may still call loginUser() again afterward for clarity.
        $crawler = $this->client->request('GET', '/join/join-peak-performance');
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Fresh',
            'player_registration[accountLastName]' => 'Player',
            'player_registration[email]' => $email,
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Fresh',
            'player_registration[playerDateOfBirth]' => sprintf('%d-01-01', ((int) date('Y')) - 22),
        ]);
        $this->client->submit($form);

        return $this->account($email);
    }
}
