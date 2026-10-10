<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\AccountStatus;
use App\Identity\Repository\UserDeletionRecordRepository;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-01.12 — Super Admin Deactivates User; US-01.13 — Super Admin Deletes
 * User (GDPR Compliance).
 */
final class AccountLifecycleTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-52: a confirmation modal states the effect; on confirmation the
     * account becomes Inactive, login is blocked.
     */
    public function testSuperAdminDeactivatesAnAccount(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('deactivate-me@example.test', AccountRole::Player);
        $this->client->loginUser($admin);

        $confirmCrawler = $this->client->request('GET', sprintf('/super-admin/users/%d/deactivate', $target->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('[role="alert"]', 'will not be able to log in');
        self::assertSelectorTextContains('[role="alert"]', 'historical data is preserved');

        $form = $confirmCrawler->selectButton('Confirm deactivation')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $reloaded = $this->account('deactivate-me@example.test');
        self::assertSame(AccountStatus::Inactive, $reloaded->getStatus());
        self::assertFalse($reloaded->isActive(), 'AC-01-52: login is blocked.');
    }

    /**
     * AC-01-52: the login attempt itself shows "Account deactivated."
     */
    public function testDeactivatedAccountCannotLogIn(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('locked-out@example.test', AccountRole::Player);
        $this->client->loginUser($admin);
        $confirmCrawler = $this->client->request('GET', sprintf('/super-admin/users/%d/deactivate', $target->getId()));
        $confirmForm = $confirmCrawler->selectButton('Confirm deactivation')->form();
        $this->client->submit($confirmForm);
        self::assertResponseRedirects();

        // /login redirects straight to the dashboard for an already
        // authenticated session (AuthController::login()) — log out first,
        // or the "Sign in" form this test needs would never render.
        $this->client->request('GET', '/logout');

        $loginCrawler = $this->client->request('GET', '/login');
        $form = $loginCrawler->selectButton('Sign in')->form([
            '_username' => 'locked-out@example.test',
            '_password' => 'password',
        ]);
        $this->client->submit($form);
        $crawler = $this->client->followRedirect();

        self::assertSelectorTextContains('[role="alert"]', 'Account deactivated');
    }

    /**
     * AC-01-53: a deactivated user still appears in the Users list, shown as
     * Inactive rather than removed.
     */
    public function testDeactivatedUserStillAppearsInTheUsersList(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('still-listed@example.test', AccountRole::Player);
        $this->deactivateViaService($admin, $target);

        $this->client->loginUser($admin);
        $this->client->request('GET', '/super-admin/users');

        self::assertSelectorTextContains('body', 'still-listed@example.test');
        self::assertSelectorTextContains('body', 'inactive');
    }

    /**
     * AC-01-54: reactivation restores Active status and login.
     */
    public function testSuperAdminReactivatesADeactivatedAccount(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('reactivate-me@example.test', AccountRole::Player);
        $this->deactivateViaService($admin, $target);

        $this->client->loginUser($admin);
        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        $form = $crawler->selectButton('Reactivate')->form();
        $this->client->submit($form);

        self::assertResponseRedirects();

        $reloaded = $this->account('reactivate-me@example.test');
        self::assertSame(AccountStatus::Active, $reloaded->getStatus());
        self::assertTrue($reloaded->isActive());
    }

    /**
     * AC-01-55: a warning confirmation modal states the effect and that it
     * cannot be undone.
     */
    public function testDeleteShowsAnExplicitCannotBeUndoneWarning(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('warned-delete@example.test', AccountRole::Player);
        $this->client->loginUser($admin);

        $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $target->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('[role="alert"]', 'cannot be undone');
        self::assertSelectorTextContains('[role="alert"]', 'Deleted User');
    }

    /**
     * AC-01-56: personal fields are anonymized (name -> "Deleted User", email
     * -> deleted_<id>@example.com, phone -> null) and status becomes Deleted.
     */
    public function testDeletingAnonymizesPersonalFieldsAndSetsStatusDeleted(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('to-delete@example.test', AccountRole::Player, phone: '555-0177');
        $targetId = $target->getId();
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $targetId));
        $form = $crawler->selectButton('Confirm permanent deletion')->form([
            'reason' => 'GDPR erasure request',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/super-admin/users');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var Account $reloaded */
        $reloaded = $em->getRepository(Account::class)->find($targetId);

        self::assertSame(sprintf('deleted_%d@example.com', $targetId), $reloaded->getEmail(), 'AC-01-56: deterministic anonymized email.');
        self::assertSame(AccountStatus::Deleted, $reloaded->getStatus());
        $profile = $reloaded->getProfile();
        self::assertNotNull($profile);
        self::assertSame('Deleted', $profile->getFirstName());
        self::assertSame('User', $profile->getLastName());
        self::assertNull($profile->getPhone(), 'AC-01-56: phone becomes NULL.');
    }

    /**
     * AC-01-57: historical records are preserved after deletion — the row
     * itself, and everything referencing it, still resolves.
     */
    public function testDeletionPreservesTheHistoricalRow(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('preserved-history@example.test', AccountRole::Player);
        $targetId = $target->getId();
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $targetId));
        $form = $crawler->selectButton('Confirm permanent deletion')->form();
        $this->client->submit($form);

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $stillThere = $em->getRepository(Account::class)->find($targetId);

        self::assertNotNull($stillThere, 'AC-01-57: the row is anonymized in place, never hard-deleted.');
        self::assertSame($targetId, $stillThere->getId(), 'AC-01-57: the same id — every foreign key pointing at it still resolves.');
    }

    /**
     * AC-01-58: a deleted account can never be reactivated.
     */
    public function testADeletedAccountCannotBeReactivated(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('deleted-no-reactivate@example.test', AccountRole::Player);
        $targetId = $target->getId();
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $targetId));
        $form = $crawler->selectButton('Confirm permanent deletion')->form();
        $this->client->submit($form);
        self::assertResponseRedirects();

        // The reactivate button must not even appear once deleted.
        $this->client->request('GET', sprintf('/super-admin/users/%d', $targetId));
        self::assertSelectorTextNotContains('body', 'Reactivate');

        // And the endpoint itself refuses.
        $this->client->request('POST', sprintf('/super-admin/users/%d/reactivate', $targetId));
        self::assertResponseStatusCodeSame(403, 'AC-01-58: reactivating a deleted account is refused.');
    }

    /**
     * AC-01-59: deletion is logged with the original user id, who deleted
     * them, when, and the reason.
     */
    public function testDeletionIsLoggedForComplianceWithReasonAndActor(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('logged-deletion@example.test', AccountRole::Player);
        $targetId = $target->getId();
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $targetId));
        $form = $crawler->selectButton('Confirm permanent deletion')->form([
            'reason' => 'Requested by user via support ticket #42',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        /** @var UserDeletionRecordRepository $records */
        $records = self::getContainer()->get(UserDeletionRecordRepository::class);
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $all = $em->getRepository(\App\Identity\Entity\UserDeletionRecord::class)->findAll();
        $record = current(array_filter($all, static fn ($r) => $r->getOriginalAccount()->getId() === $targetId));

        self::assertNotFalse($record, 'AC-01-59: the deletion is logged.');
        self::assertSame($targetId, $record->getOriginalAccount()->getId(), 'AC-01-59: original user id.');
        self::assertSame('logged-deletion@example.test', $record->getOriginalEmail());
        self::assertSame('admin@practiceperfect.test', $record->getDeletedByAccount()->getEmail(), 'AC-01-59: who deleted them.');
        self::assertSame('Requested by user via support ticket #42', $record->getReason(), 'AC-01-59: the reason.');
        self::assertLessThanOrEqual(new \DateTimeImmutable(), $record->getDeletedAt(), 'AC-01-59: when.');
        unset($records);
    }

    private function createAccount(string $email, AccountRole $role, ?string $phone = null): Account
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);

        $account = new Account($email, '', $role);
        $account->changePasswordHash($hasher->hashPassword($account, 'password'));
        $account->verifyEmail();
        $em->persist($account);
        $em->persist(new AccountProfile($account, 'Test', 'User', $phone));
        $em->flush();

        return $account;
    }

    private function deactivateViaService(Account $admin, Account $target): void
    {
        /** @var \App\Identity\Service\AccountLifecycleService $service */
        $service = self::getContainer()->get(\App\Identity\Service\AccountLifecycleService::class);
        $service->deactivate($admin, $target);
    }
}
