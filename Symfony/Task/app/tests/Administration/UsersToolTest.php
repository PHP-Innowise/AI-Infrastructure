<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * AC-01-71 (Super Admin can edit any account) and AC-01-72 (the Users tool's
 * own search and filters).
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-71, AC-01-72
 */
final class UsersToolTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-71: every editable field on the form is actually applied,
     * including email — not merely name/phone.
     */
    public function testSuperAdminEditsAnyAccountIncludingItsEmail(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('before-edit@example.test', AccountRole::Player);
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save account')->form([
            'admin_edit_account[firstName]' => 'Edited',
            'admin_edit_account[lastName]' => 'Person',
            'admin_edit_account[email]' => 'after-edit@example.test',
            'admin_edit_account[phone]' => '555-0199',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/super-admin/users/%d', $target->getId()));
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Account updated');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->clear();
        /** @var Account $reloaded */
        $reloaded = $em->getRepository(Account::class)->find($target->getId());

        self::assertSame('after-edit@example.test', $reloaded->getEmail(), 'AC-01-71: email is actually changed, not silently dropped.');
        $profile = $reloaded->getProfile();
        self::assertNotNull($profile);
        self::assertSame('Edited', $profile->getFirstName());
        self::assertSame('Person', $profile->getLastName());
        self::assertSame('555-0199', $profile->getPhone());
    }

    /**
     * AC-01-71 combined with BR-01-2 (email uniqueness): editing an account
     * to an email already held by someone else is refused with a clear form
     * error, and the target's email is left unchanged.
     */
    public function testEditingToAnAlreadyUsedEmailShowsAFormErrorAndChangesNothing(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $this->createAccount('taken@example.test', AccountRole::Player);
        $target = $this->createAccount('still-free@example.test', AccountRole::Player);
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));
        $form = $crawler->selectButton('Save account')->form([
            'admin_edit_account[firstName]' => 'Test',
            'admin_edit_account[lastName]' => 'User',
            'admin_edit_account[email]' => 'taken@example.test',
        ]);
        $this->client->submit($form);

        self::assertSelectorTextContains('body', 'already exists');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->clear();
        /** @var Account $reloaded */
        $reloaded = $em->getRepository(Account::class)->find($target->getId());
        self::assertSame('still-free@example.test', $reloaded->getEmail(), 'The rejected edit must not have changed the email.');
    }

    /**
     * AC-01-72: "q" searches name/email — tool-specific, not a global search.
     */
    public function testUsersToolSearchMatchesNameOrEmail(): void
    {
        $this->createAccount('zzz-distinctive@example.test', AccountRole::Player);
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', '/super-admin/users', ['q' => 'zzz-distinctive']);
        self::assertSelectorTextContains('body', 'zzz-distinctive@example.test');

        $this->client->request('GET', '/super-admin/users', ['q' => 'no-such-substring-exists']);
        self::assertSelectorTextNotContains('body', 'zzz-distinctive@example.test');
    }

    /**
     * AC-01-72: the role filter.
     */
    public function testUsersToolFiltersByRole(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', '/super-admin/users', ['role' => AccountRole::Coach->value]);

        self::assertSelectorTextContains('body', 'coach@practiceperfect.test');
        self::assertSelectorTextNotContains('body', 'trainer@practiceperfect.test');
    }

    /**
     * AC-01-72: the status filter.
     */
    public function testUsersToolFiltersByStatus(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->createAccount('status-filter-target@example.test', AccountRole::Player);
        /** @var \App\Identity\Service\AccountLifecycleService $lifecycle */
        $lifecycle = self::getContainer()->get(\App\Identity\Service\AccountLifecycleService::class);
        $lifecycle->deactivate($admin, $target);

        $this->client->loginUser($admin);

        $this->client->request('GET', '/super-admin/users', ['status' => 'inactive']);
        self::assertSelectorTextContains('body', 'status-filter-target@example.test');

        $this->client->request('GET', '/super-admin/users', ['status' => 'active']);
        self::assertSelectorTextNotContains('body', 'status-filter-target@example.test');
    }

    private function createAccount(string $email, AccountRole $role): Account
    {
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);

        $account = new Account($email, '', $role);
        $account->changePasswordHash($hasher->hashPassword($account, 'password'));
        $em->persist($account);
        $em->persist(new AccountProfile($account, 'Test', 'User'));
        $em->flush();

        return $account;
    }
}
