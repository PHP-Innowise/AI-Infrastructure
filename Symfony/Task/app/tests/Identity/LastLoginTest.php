<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Tests\Support\FixtureHelpers;
use Doctrine\DBAL\Connection;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * AC-07-10: the Users tool's table shows "last login".
 *
 * The column shipped, `account.last_login_at` shipped, and
 * `Account::recordLogin()` shipped with no caller anywhere — so every row read
 * "Never", including accounts that had signed in a minute earlier. Manual
 * testing found it that way, which matters for the screen it is on: an admin
 * deciding whether an account is abandoned was reading a field that could only
 * ever say "never".
 *
 * These assert the timestamp through a real sign-in, not through the setter.
 */
final class LastLoginTest extends WebTestCase
{
    use FixtureHelpers;

    private const string PASSWORD = 'a-password-only-this-test-uses';

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testSigningInRecordsTheTimestamp(): void
    {
        $email = $this->createSignInAccount();
        self::assertNull($this->lastLoginOf($email), 'Precondition: never signed in.');

        $before = new \DateTimeImmutable();
        $this->signIn($email);

        $recorded = $this->lastLoginOf($email);
        self::assertNotNull($recorded, 'A successful sign-in must leave a trace on the account.');
        self::assertGreaterThanOrEqual(
            $before->modify('-5 seconds')->getTimestamp(),
            (new \DateTimeImmutable($recorded))->getTimestamp(),
            'The recorded time is this sign-in, not an older one.',
        );
    }

    /**
     * A second sign-in moves it — "last", not "first".
     */
    public function testASubsequentSignInMovesTheTimestampForward(): void
    {
        $email = $this->createSignInAccount();
        $this->signIn($email);

        // Backdated rather than waiting a second: the assertion is that the
        // value is rewritten, and a clock is a poor thing to make a test
        // depend on.
        $this->connection()->executeStatement(
            "UPDATE account SET last_login_at = now() - interval '3 days' WHERE email = ?",
            [$email],
        );
        $backdated = $this->lastLoginOf($email);
        self::assertNotNull($backdated);

        $this->client->request('GET', '/logout');
        $this->signIn($email);

        $current = $this->lastLoginOf($email);
        self::assertNotNull($current);
        self::assertGreaterThan(
            (new \DateTimeImmutable($backdated))->getTimestamp(),
            (new \DateTimeImmutable($current))->getTimestamp(),
        );
    }

    /**
     * A failed attempt is not a login. Recording one would make the column
     * answer a different question than the one an admin is asking.
     */
    public function testAFailedAttemptRecordsNothing(): void
    {
        $email = $this->createSignInAccount();

        $crawler = $this->client->request('GET', '/login');
        $this->client->submit($crawler->selectButton('Sign in')->form([
            '_username' => $email,
            '_password' => 'not-the-password',
        ]));

        self::assertNull($this->lastLoginOf($email));
    }

    /**
     * Impersonation is not a login either, and this is the case worth pinning:
     * Symfony's switch_user swaps the token without going through the
     * authenticator manager, so the event this listens for never fires. An
     * admin looking at a parent's portal must not rewrite that parent's
     * last-login date on the very screen used to judge whether the account is
     * still in use — the audit log's impersonation_start is where that visit
     * belongs.
     */
    public function testImpersonatingSomeoneDoesNotCountAsThatPersonSigningIn(): void
    {
        $target = 'player@practiceperfect.test';
        $this->clearLastLogin($target);

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $this->client->request('GET', '/dashboard?_switch_user='.$target);
        $this->client->followRedirect();

        self::assertSelectorTextContains('.impersonation-banner', $target, 'Precondition: the swap happened.');
        self::assertNull($this->lastLoginOf($target), 'Being impersonated is not signing in.');
    }

    /**
     * And the screen the criterion is actually about.
     */
    public function testTheUsersToolShowsTheRecordedTimeInsteadOfNever(): void
    {
        $email = $this->createSignInAccount();
        $this->signIn($email);
        $this->client->request('GET', '/logout');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/users?q='.urlencode($email));

        self::assertResponseIsSuccessful();

        $row = $crawler->filter('table tbody tr')->reduce(
            static fn ($node): bool => str_contains($node->text(), $email),
        );
        self::assertCount(1, $row, 'The searched account is on the page.');

        $recorded = $this->lastLoginOf($email);
        self::assertNotNull($recorded);
        self::assertStringContainsString(
            (new \DateTimeImmutable($recorded))->format('Y-m-d'),
            $row->text(),
            'AC-07-10: the row shows when they last signed in, not "Never".',
        );
    }

    /**
     * A fresh account with a password only this test knows.
     *
     * The obvious choice — a fixture account and the fixture password — is
     * not safe here: the suite shares one mutable database, and the password
     * reset tests genuinely change what `trainer@` and `coach@` sign in with.
     * Borrowing one of those would make this test's result depend on
     * execution order, which is exactly the kind of flake that teaches people
     * to ignore a red suite.
     */
    private function createSignInAccount(): string
    {
        $email = sprintf('last-login-%s@example.test', uniqid());

        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);

        $account = new Account($email, '', AccountRole::Player);
        $account->changePasswordHash($hasher->hashPassword($account, self::PASSWORD));
        $entityManager->persist($account);
        $entityManager->persist(new AccountProfile($account, 'Last', 'Login'));
        $entityManager->flush();

        return $email;
    }

    private function signIn(string $email): void
    {
        $crawler = $this->client->request('GET', '/login');
        $this->client->submit($crawler->selectButton('Sign in')->form([
            '_username' => $email,
            '_password' => self::PASSWORD,
        ]));
        $this->client->followRedirect();
    }

    private function clearLastLogin(string $email): void
    {
        $this->connection()->executeStatement('UPDATE account SET last_login_at = NULL WHERE email = ?', [$email]);
    }

    private function lastLoginOf(string $email): ?string
    {
        $value = $this->connection()->fetchOne('SELECT last_login_at FROM account WHERE email = ?', [$email]);

        return \is_string($value) ? $value : null;
    }

    private function connection(): Connection
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        return $connection;
    }
}
