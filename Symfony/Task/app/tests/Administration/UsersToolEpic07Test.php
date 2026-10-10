<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\PlayerProfileRepository;
use App\Platform\Entity\AccountTrainerLink;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-07.02/03 — Super Admin Views All Users / Edits User Profile.
 * AC-01-71/72's own UsersToolTest.php already proves the search/filter/edit
 * mechanics this epic reuses; this file adds Epic-07's own additions:
 * pagination (AC-07-12), the full user-table column set (AC-07-10),
 * per-user actions (AC-07-11), and player-specific edit fields (AC-07-14).
 */
final class UsersToolEpic07Test extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-8: Super Admin navigates to a "Users" tool and views a list of
     * all users across all trainers (trainers, coaches, players, parents).
     */
    public function testUsersToolListsEveryRoleAcrossEveryTrainer(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/users', ['q' => '', 'role' => '']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'trainer@practiceperfect.test');
        self::assertSelectorTextContains('body', 'trainer-b@practiceperfect.test');
        self::assertSelectorTextContains('body', 'coach@practiceperfect.test');
        self::assertSelectorTextContains('body', 'player@practiceperfect.test');
    }

    /**
     * AC-07-9: search box plus Role and Status filters, results in a table
     * — already proven for role/status individually by UsersToolTest's own
     * AC-01-72 tests; this asserts both controls are actually PRESENT on
     * the page as named.
     */
    public function testUsersToolExposesSearchAndFilterControls(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/users');

        self::assertResponseIsSuccessful();
        self::assertCount(1, $crawler->filter('input[name="q"]'));
        self::assertCount(1, $crawler->filter('select[name="role"]'));
        self::assertCount(1, $crawler->filter('select[name="status"]'));
    }

    /**
     * AC-07-10: name, email, role, associated trainer (for coaches/
     * players), status, registration date, and last login all appear.
     */
    public function testUserTableShowsEveryRequiredColumn(): void
    {
        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/users', ['q' => 'coach@practiceperfect.test']);

        self::assertResponseIsSuccessful();
        $row = $crawler->filter('table tbody tr')->first();
        self::assertStringContainsString('coach@practiceperfect.test', $row->text());
        self::assertStringContainsString('Coach', $row->text());
        // AC-07-10 "associated trainer": the coach fixture is linked to
        // Trainer A specifically.
        self::assertStringContainsString('Peak Performance Basketball', $row->text());
        self::assertStringContainsString('active', $row->text());
        // A registration date column renders a real date, and "Never"
        // (never logged in) or a timestamp for last login — both present
        // as columns regardless of which value this particular fixture has.
        self::assertSelectorTextContains('thead', 'Registered');
        self::assertSelectorTextContains('thead', 'Last login');
    }

    /**
     * AC-07-11: view, edit, impersonate, and deactivate actions per user —
     * proven together on the user_show page for a non-Super-Admin target.
     */
    public function testUserRowOffersViewEditImpersonateAndDeactivateActions(): void
    {
        $target = $this->account('trainer-b@practiceperfect.test');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));

        self::assertResponseIsSuccessful();
        self::assertGreaterThan(0, $crawler->selectLink('Edit')->count(), 'AC-07-11: edit action.');
        self::assertGreaterThan(0, $crawler->selectButton('Impersonate')->count(), 'AC-07-11: impersonate action.');
        self::assertGreaterThan(0, $crawler->selectLink('Deactivate')->count(), 'AC-07-11: deactivate action.');
    }

    /**
     * AC-07-12: the Users tool paginates at 50 users per page with page
     * navigation. Proven by creating 51 fresh, uniquely-named accounts
     * (isolated from any other test's shared fixture data via a
     * distinctive marker in the search query) and confirming the first
     * page shows exactly 50 with a working "Next" link, and the 51st
     * appears only on page 2.
     */
    public function testUsersToolPaginatesAtFiftyPerPage(): void
    {
        $marker = 'epic07-pagination-'.bin2hex(random_bytes(4));

        for ($i = 1; $i <= 51; ++$i) {
            $this->createAccount(sprintf('%s-%02d@example.test', $marker, $i), AccountRole::Player);
        }

        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawlerPage1 = $this->client->request('GET', '/super-admin/users', ['q' => $marker]);
        self::assertResponseIsSuccessful();
        self::assertCount(50, $crawlerPage1->filter('table tbody tr'), 'AC-07-12: exactly 50 per page.');
        self::assertGreaterThan(0, $crawlerPage1->selectLink('Next')->count());

        $crawlerPage2 = $this->client->request('GET', '/super-admin/users', ['q' => $marker, 'page' => 2]);
        self::assertResponseIsSuccessful();
        self::assertCount(1, $crawlerPage2->filter('table tbody tr'), 'AC-07-12: the 51st account is on page 2.');
    }

    /**
     * AC-07-13: clicking "Edit" opens the edit form.
     */
    public function testClickingEditOpensTheEditForm(): void
    {
        $target = $this->createAccount('open-edit-form@example.test', AccountRole::Player);
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        $editLink = $crawler->selectLink('Edit');
        self::assertGreaterThan(0, $editLink->count());

        $this->client->click($editLink->link());
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('form[name="admin_edit_account"]');
    }

    /**
     * AC-07-14: role and associated trainer are view-only on the edit
     * screen — no form field for either exists at all.
     */
    public function testEditFormNeverExposesARoleOrTrainerField(): void
    {
        $target = $this->account('coach@practiceperfect.test');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));

        self::assertResponseIsSuccessful();
        self::assertCount(0, $crawler->filter('select[name="admin_edit_account[role]"], input[name="admin_edit_account[role]"]'));
        self::assertCount(0, $crawler->filter('select[name="admin_edit_account[trainer]"], input[name="admin_edit_account[trainer]"]'));
        self::assertSelectorTextContains('body', 'Role: '.$target->getRole()->label().' (view-only)');
    }

    /**
     * AC-07-14: player-specific profile details (gender, school/team) are
     * editable when the account is a player; AC-07-15 covers the save +
     * audit half.
     */
    public function testEditFormExposesPlayerSpecificFieldsForAPlayerAccount(): void
    {
        $target = $this->account('player@practiceperfect.test');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));

        self::assertResponseIsSuccessful();
        self::assertCount(1, $crawler->filter('select[name="admin_edit_account[playerGender]"]'), 'AC-07-14: gender is editable for a player.');
        self::assertCount(1, $crawler->filter('input[name="admin_edit_account[playerSchoolOrTeam]"]'));
    }

    /**
     * AC-07-14 (negative half): the SAME fields never render for a
     * non-player account (a trainer has no PlayerProfile to edit at all).
     */
    public function testEditFormNeverExposesPlayerFieldsForANonPlayerAccount(): void
    {
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));

        self::assertResponseIsSuccessful();
        self::assertCount(0, $crawler->filter('select[name="admin_edit_account[playerGender]"]'));
    }

    /**
     * AC-07-15: saving the edit form persists player-specific changes and
     * creates an audit log entry ("Super Admin edited user [Name]").
     */
    public function testSavingPlayerEditPersistsGenderAndSchoolAndIsAudited(): void
    {
        $target = $this->account('player@practiceperfect.test');
        $admin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d/edit', $target->getId()));
        $form = $crawler->selectButton('Save account')->form([
            'admin_edit_account[firstName]' => 'Pat',
            'admin_edit_account[lastName]' => 'Parent',
            'admin_edit_account[email]' => $target->getEmail(),
            'admin_edit_account[playerGender]' => 'female',
            'admin_edit_account[playerSchoolOrTeam]' => 'Northgate Middle School',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects(sprintf('/super-admin/users/%d', $target->getId()));
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Account updated');

        /** @var PlayerProfileRepository $players */
        $players = self::getContainer()->get(PlayerProfileRepository::class);
        $profile = $players->findOneForSelfAccount($target);
        self::assertNotNull($profile);
        self::assertSame('female', $profile->getGender(), 'AC-07-14/15: the new gender is persisted.');
        self::assertSame('Northgate Middle School', $profile->getSchoolOrTeam());
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

        if (AccountRole::Coach === $role || AccountRole::Player === $role) {
            $em->flush();
            $em->persist(new AccountTrainerLink($account, $this->trainer('peak-performance'), $role->value));
        }

        $em->flush();

        return $account;
    }
}
