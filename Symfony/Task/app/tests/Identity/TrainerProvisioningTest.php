<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Identity\Service\TrainerProvisioningService;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-01.01 — Super Admin Creates Trainer Account.
 *
 * MailerAssertionsTrait is already pulled in by KernelTestCase, which
 * WebTestCase extends — no separate trait import needed.
 */
final class TrainerProvisioningTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-1: created from the Users tool; AC-01-2: captures business name,
     * trainer name, email, phone; AC-01-6: appears with status Active;
     * AC-01-73 (player half): a static, unlimited-use, no-expiry ShareLink
     * is provisioned for the trainer from day one.
     */
    public function testSuperAdminCreatesTrainerAccountViaTheUsersTool(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => 'Summit Sports Academy',
            'create_trainer[trainerFirstName]' => 'Sam',
            'create_trainer[trainerLastName]' => 'Summit',
            'create_trainer[email]' => 'sam.summit@example.test',
            'create_trainer[phone]' => '555-0100',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertResponseIsSuccessful();

        $account = $this->account('sam.summit@example.test');
        self::assertSame(AccountRole::Trainer, $account->getRole());
        self::assertSame(AccountStatus::Active, $account->getStatus(), 'AC-01-6: the new trainer account has status Active.');
        self::assertNotNull($account->getProfile());
        self::assertSame('Sam', $account->getProfile()->getFirstName());
        self::assertSame('Summit', $account->getProfile()->getLastName());
        self::assertSame('555-0100', $account->getProfile()->getPhone());

        $trainer = $this->trainer('summit-sports-academy');
        self::assertSame('Summit Sports Academy', $trainer->getBusinessName(), 'AC-01-2: business name is captured.');
        self::assertSame($account->getId(), $trainer->getOwnerAccount()->getId());

        $this->activateTenant($trainer);
        /** @var ShareLinkRepository $shareLinks */
        $shareLinks = self::getContainer()->get(ShareLinkRepository::class);
        $staticLink = $shareLinks->findStaticPlayerLink((int) $trainer->getId());
        self::assertNotNull($staticLink, 'AC-01-73: a static player ShareLink is generated for every new trainer.');
        self::assertNull($staticLink->getExpiresAt(), 'AC-01-73: the static player link never expires.');
        self::assertNull($staticLink->getMaxUses(), 'AC-01-73: the static player link is unlimited-use.');
    }

    /**
     * AC-01-3: a temporary password / setup link is generated. AC-01-4: a
     * setup email with login credentials/instructions is sent.
     */
    public function testCreatingATrainerSendsASetupEmail(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => 'Northgate Volleyball',
            'create_trainer[trainerFirstName]' => 'Nora',
            'create_trainer[trainerLastName]' => 'North',
            'create_trainer[email]' => 'nora.north@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        // config/packages/messenger.yaml routes SendEmailMessage through the
        // async transport, so this email is queued, not sent synchronously —
        // assertQueuedEmailCount, not assertEmailCount.
        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'nora.north@example.test');
        self::assertEmailTextBodyContains($email, 'Set your password');
    }

    /**
     * AC-01-7: email must be unique; AC-01-8: a duplicate shows a clear error.
     */
    public function testDuplicateEmailIsRejectedWithAClearError(): void
    {
        /** @var TrainerProvisioningService $service */
        $service = self::getContainer()->get(TrainerProvisioningService::class);
        $admin = $this->account('admin@practiceperfect.test');

        $this->expectException(DuplicateEmailException::class);

        // trainer@practiceperfect.test already exists in the fixtures.
        $service->createTrainer($admin, 'Duplicate Co', 'Dupe', 'Trainer', 'trainer@practiceperfect.test', null);
    }

    /**
     * AC-01-8, at the HTTP layer: the form re-renders with a field-level
     * error rather than a generic failure or a raw exception page.
     */
    public function testDuplicateEmailShowsAFormErrorInsteadOfCrashing(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => 'Second Attempt Co',
            'create_trainer[trainerFirstName]' => 'Sec',
            'create_trainer[trainerLastName]' => 'Ond',
            'create_trainer[email]' => 'trainer@practiceperfect.test',
        ]);
        $this->client->submit($form);

        // Symfony renders an invalid-form response as 422, not 200 — still
        // "not crashing" (no exception page), which is what AC-01-8 asks for.
        self::assertResponseStatusCodeSame(422);
        self::assertSelectorTextContains('#create_trainer_email_error1', 'already exists');
    }

    /**
     * AC-01-7: required fields are enforced (business name here).
     */
    public function testRequiredFieldsAreEnforcedOnTrainerCreation(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/super-admin/trainers/new');
        $form = $crawler->selectButton('Create trainer account')->form([
            'create_trainer[businessName]' => '',
            'create_trainer[trainerFirstName]' => 'No',
            'create_trainer[trainerLastName]' => 'Business',
            'create_trainer[email]' => 'no-business-name@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422);
        self::assertNull($this->maybeAccount('no-business-name@example.test'), 'No account should be created when a required field is blank.');
    }

    /**
     * Only a Super Admin reaches this tool at all — BR-01-13's enforcement
     * boundary from the other side.
     */
    public function testOnlySuperAdminCanCreateATrainer(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $this->client->request('GET', '/super-admin/trainers/new');

        self::assertResponseStatusCodeSame(403);
    }

    private function maybeAccount(string $email): ?Account
    {
        /** @var AccountRepository $repository */
        $repository = self::getContainer()->get(AccountRepository::class);

        return $repository->findOneByEmail($email);
    }
}
