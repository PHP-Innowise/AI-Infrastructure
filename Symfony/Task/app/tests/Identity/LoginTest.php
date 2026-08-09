<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\AccountRepository;
use PHPUnit\Framework\Attributes\DataProvider;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\HttpFoundation\Response;

/**
 * Epic-01 authentication.
 *
 * AC-01-5  — a trainer can log in and reach the trainer dashboard.
 * AC-01-36 — a registered user can log in with email and password.
 * AC-01-37 — invalid credentials are rejected with a generic message.
 */
final class LoginTest extends WebTestCase
{
    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-36: every MVP role reaches its dashboard with correct credentials.
     * This is the criterion "all four MVP roles can log in" is measured by.
     */
    #[DataProvider('mvpRoleProvider')]
    public function testEachMvpRoleCanLogIn(string $email, AccountRole $expectedRole): void
    {
        $account = $this->accountFor($email);

        self::assertSame($expectedRole, $account->getRole(), 'Fixture role drifted from the expected MVP role.');

        $this->client->loginUser($account);
        $this->client->request('GET', '/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Welcome');
    }

    /**
     * @return iterable<string, array{string, AccountRole}>
     */
    public static function mvpRoleProvider(): iterable
    {
        yield 'super admin' => ['admin@practiceperfect.test', AccountRole::SuperAdmin];
        yield 'trainer' => ['trainer@practiceperfect.test', AccountRole::Trainer];
        yield 'coach' => ['coach@practiceperfect.test', AccountRole::Coach];
        yield 'player/parent' => ['player@practiceperfect.test', AccountRole::Player];
    }

    /**
     * AC-01-36: the real form-login path, not just programmatic login, so the
     * firewall, CSRF token and password hasher are all exercised.
     */
    public function testTrainerCanLogInThroughTheLoginForm(): void
    {
        $crawler = $this->client->request('GET', '/login');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Sign in')->form([
            '_username' => 'trainer@practiceperfect.test',
            '_password' => 'password',
        ]);

        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'trainer@practiceperfect.test');
    }

    /**
     * AC-01-37. The message must not distinguish "no such account" from "wrong
     * password": a form that does is an account-enumeration oracle.
     */
    public function testInvalidCredentialsAreRejectedWithoutRevealingWhichPartWasWrong(): void
    {
        $crawler = $this->client->request('GET', '/login');

        $form = $crawler->selectButton('Sign in')->form([
            '_username' => 'trainer@practiceperfect.test',
            '_password' => 'not-the-password',
        ]);

        $this->client->submit($form);
        $crawler = $this->client->followRedirect();

        self::assertSelectorTextContains('[role="alert"]', 'Invalid credentials');

        $bodyText = $crawler->filter('body')->text();
        self::assertStringNotContainsStringIgnoringCase('no such user', $bodyText);
        self::assertStringNotContainsStringIgnoringCase('unknown email', $bodyText);
    }

    /**
     * BR-01-2: email is CITEXT, so a differently-cased address is the same
     * account rather than a second one.
     */
    public function testEmailLookupIsCaseInsensitive(): void
    {
        $lower = $this->accountFor('trainer@practiceperfect.test');
        $upper = $this->accountFor('TRAINER@PRACTICEPERFECT.TEST');

        self::assertSame($lower->getId(), $upper->getId());
    }

    public function testAnonymousUserIsRedirectedAwayFromTheDashboard(): void
    {
        $this->client->request('GET', '/dashboard');

        self::assertResponseStatusCodeSame(Response::HTTP_FOUND);
        self::assertResponseHeaderSame('location', 'http://localhost/login');
    }

    private function accountFor(string $email): Account
    {
        /** @var AccountRepository $repository */
        $repository = self::getContainer()->get(AccountRepository::class);
        $account = $repository->findOneByEmail($email);

        self::assertInstanceOf(Account::class, $account, sprintf('Fixture account "%s" is missing.', $email));

        return $account;
    }
}
