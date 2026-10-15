<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.11 — User Edits Own Profile.
 */
final class ProfileEditTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-48: common fields are editable; email, role, and (for players)
     * skill level stay read-only — never even present as bindable fields.
     */
    public function testCommonFieldsAreEditableAndEmailRoleStayReadOnly(): void
    {
        $trainer = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainer);

        $crawler = $this->client->request('GET', '/account/profile');
        self::assertResponseIsSuccessful();

        self::assertSelectorTextContains('body', 'trainer@practiceperfect.test');
        self::assertSelectorNotExists('input[name="edit_profile[email]"]', 'AC-01-48: email is never a bindable field.');
        self::assertSelectorNotExists('select[name="edit_profile[role]"]', 'AC-01-48: role is never a bindable field.');

        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Tina Updated',
            'edit_profile[lastName]' => 'Trainer',
            'edit_profile[phone]' => '555-0199',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $reloaded = $this->account('trainer@practiceperfect.test');
        $profile = $reloaded->getProfile();
        self::assertNotNull($profile);
        self::assertSame('Tina Updated', $profile->getFirstName());
        self::assertSame('555-0199', $profile->getPhone());
        self::assertSame('trainer@practiceperfect.test', $reloaded->getEmail(), 'AC-01-48: email is unchanged — it was never on the form.');
    }

    /**
     * AC-01-49: saving persists changes with a confirmation message.
     */
    public function testSavingShowsAConfirmationMessage(): void
    {
        $coach = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($coach);

        $crawler = $this->client->request('GET', '/account/profile');
        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Casey',
            'edit_profile[lastName]' => 'Coach',
        ]);
        $this->client->submit($form);

        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Profile saved');
    }

    /**
     * AC-01-50: phone number format is validated; required fields enforced.
     */
    public function testPhoneFormatIsValidatedAndRequiredFieldsEnforced(): void
    {
        $player = $this->account('player@practiceperfect.test');
        $this->client->loginUser($player);

        $crawler = $this->client->request('GET', '/account/profile');
        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Pat',
            'edit_profile[lastName]' => 'Parent',
            'edit_profile[phone]' => 'not-a-phone-number!!',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422, 'AC-01-50: an invalid phone format is rejected.');

        $crawler2 = $this->client->request('GET', '/account/profile');
        $form2 = $crawler2->selectButton('Save changes')->form([
            'edit_profile[firstName]' => '',
            'edit_profile[lastName]' => 'Parent',
        ]);
        $this->client->submit($form2);

        self::assertResponseStatusCodeSame(422, 'AC-01-50: first name is required.');
    }

    /**
     * AC-01-51: role-specific fields apply on top of the common set —
     * Player (school, jersey), Coach (bio/credentials/certifications/public
     * checkbox), Trainer (organization details).
     */
    public function testRoleSpecificFieldsApplyOnTopOfTheCommonSet(): void
    {
        // Player.
        $player = $this->account('player@practiceperfect.test');
        $this->client->loginUser($player);
        $crawler = $this->client->request('GET', '/account/profile');
        self::assertSelectorExists('input[name="edit_profile[schoolOrTeam]"]');
        self::assertSelectorExists('input[name="edit_profile[jerseyNumber]"]');
        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Pat',
            'edit_profile[lastName]' => 'Parent',
            'edit_profile[schoolOrTeam]' => 'Riverside High',
            'edit_profile[jerseyNumber]' => '7',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $selfPlayer = $playerProfiles->findOneForSelfAccount($this->account('player@practiceperfect.test'));
        self::assertNotNull($selfPlayer);
        self::assertSame('Riverside High', $selfPlayer->getSchoolOrTeam(), 'AC-01-51: Player role-specific field (school).');
        self::assertSame('7', $selfPlayer->getJerseyNumber(), 'AC-01-51: Player role-specific field (jersey number).');

        // Coach.
        $coach = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($coach);
        $coachCrawler = $this->client->request('GET', '/account/profile');
        self::assertSelectorExists('textarea[name="edit_profile[bio]"]');
        $coachForm = $coachCrawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Casey',
            'edit_profile[lastName]' => 'Coach',
            'edit_profile[bio]' => 'Ten years of coaching experience.',
            'edit_profile[isPublicProfile]' => true,
        ]);
        $this->client->submit($coachForm);
        self::assertResponseRedirects();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var CoachMembershipRepository $coachMemberships */
        $coachMemberships = self::getContainer()->get(CoachMembershipRepository::class);
        $membership = $coachMemberships->findOneForAccountInActiveTenant($this->account('coach@practiceperfect.test'));
        self::assertNotNull($membership);
        self::assertSame('Ten years of coaching experience.', $membership->getBio(), 'AC-01-51: Coach role-specific field (bio).');
        self::assertTrue($membership->isPublicProfile(), 'AC-01-51: Coach public-profile checkbox.');

        // Trainer.
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainerAccount);
        $trainerCrawler = $this->client->request('GET', '/account/profile');
        self::assertSelectorExists('input[name="edit_profile[organizationWebsite]"]');
        $trainerForm = $trainerCrawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Tina',
            'edit_profile[lastName]' => 'Trainer',
            'edit_profile[organizationAddress]' => '1 Main St',
            'edit_profile[organizationWebsite]' => 'https://example.test',
        ]);
        $this->client->submit($trainerForm);
        self::assertResponseRedirects();

        $reloadedTrainer = $this->trainer('peak-performance');
        self::assertSame('1 Main St', $reloadedTrainer->getOrganizationAddress(), 'AC-01-51: Trainer role-specific field (organization details).');
    }
}
