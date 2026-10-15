<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\Account;
use App\Identity\Entity\EmailVerificationToken;
use App\Identity\Repository\EmailVerificationTokenRepository;
use App\Platform\Service\SecureTokenFactory;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * AC-01-67: email verification sends and processes correctly.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-67, BR-01-5
 */
final class EmailVerificationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-67, end to end: registering queues a verification email, the
     * account starts unverified, and following the emailed link marks it
     * verified.
     */
    public function testRegisteringQueuesAVerificationEmailAndTheLinkVerifiesTheAccount(): void
    {
        $this->registerFreshPlayer('verify-me@example.test');

        $account = $this->account('verify-me@example.test');
        self::assertFalse($account->isEmailVerified(), 'A newly registered account starts unverified.');

        $verificationEmail = $this->findQueuedEmailContaining('/email/verify/');
        self::assertNotNull($verificationEmail, 'A verification email should have been queued at registration.');

        $token = $this->extractToken((string) $verificationEmail->getTextBody(), '/email/verify/');

        $this->client->request('GET', '/email/verify/'.$token);
        self::assertResponseRedirects('/login');
        // registerFreshPlayer() leaves the client authenticated (registering
        // auto-logs in), so /login itself immediately redirects again to
        // /dashboard (AuthController::login()'s already-authenticated
        // branch) rather than rendering — a second hop is needed to reach a
        // page that actually renders the flash.
        $this->client->followRedirect();
        $this->client->followRedirect();
        // Not scoped to the first `.flash--success` node: registration's own
        // still-pending "Welcome to ..." flash (never consumed, since
        // registerFreshPlayer() does not follow its redirect) renders first
        // in the same flash list, ahead of this one.
        self::assertSelectorExists('.flash--success');
        self::assertSelectorTextContains('body', 'Your email address has been verified.');

        $reVerified = $this->account('verify-me@example.test');
        self::assertTrue($reVerified->isEmailVerified(), 'AC-01-67: the account is verified once the link is followed.');
    }

    /**
     * BR-01-5: a verification token is single-use, same as password reset.
     */
    public function testAnAlreadyConsumedVerificationTokenCannotBeReused(): void
    {
        $token = $this->issueRawToken('replay-verify@example.test');

        $this->client->request('GET', '/email/verify/'.$token);
        self::assertResponseRedirects('/login');

        $this->client->request('GET', '/email/verify/'.$token);
        self::assertResponseStatusCodeSame(404);
        self::assertSelectorTextContains('h1', 'invalid or has expired');
    }

    /**
     * BR-01-5: 24-hour expiry.
     */
    public function testAnExpiredVerificationTokenIsRejected(): void
    {
        $account = $this->registerFreshPlayer('expired-verify@example.test');

        /** @var SecureTokenFactory $tokenFactory */
        $tokenFactory = self::getContainer()->get(SecureTokenFactory::class);
        $generated = $tokenFactory->generate();
        $token = new EmailVerificationToken($account, $generated->hash);

        /** @var EmailVerificationTokenRepository $tokens */
        $tokens = self::getContainer()->get(EmailVerificationTokenRepository::class);
        $tokens->add($token);
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->flush();

        $reflection = new \ReflectionProperty($token, 'expiresAt');
        $reflection->setAccessible(true);
        $reflection->setValue($token, new \DateTimeImmutable('-1 minute'));
        $em->flush();

        $this->client->request('GET', '/email/verify/'.$generated->raw);
        self::assertResponseStatusCodeSame(404);
    }

    /**
     * The dashboard surfaces a "resend" action while unverified (this test
     * exercises it, closing what was otherwise a route with no UI trigger).
     */
    public function testLoggedInUserCanResendTheirOwnVerificationEmail(): void
    {
        $this->registerFreshPlayer('resend-verify@example.test');
        // Registration already leaves the client logged in as the new
        // account (see ShareLinkController::register()).

        $crawler = $this->client->request('GET', '/dashboard');

        $form = $crawler->selectButton('Resend verification email')->form();
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        // The mailer assertions read the CURRENT request's
        // mailer.message_logger_listener, a service that is rebuilt fresh
        // on every kernel reboot — so this reflects only messages queued by
        // the resend request just made, not an accumulation across the
        // earlier registration request too. Check it now, before
        // followRedirect() triggers another reboot and a fresh, empty view.
        self::assertQueuedEmailCount(1, message: 'AC-01-67: resending queues another verification email.');
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'resend-verify@example.test');

        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'Verification email sent');
    }

    /**
     * The resend endpoint is a state-changing POST behind
     * IS_AUTHENTICATED_FULLY — an anonymous caller is redirected to log in,
     * never allowed to trigger a send for an account they do not hold.
     */
    public function testResendingRequiresAuthentication(): void
    {
        $this->client->request('POST', '/email/verify/resend');

        self::assertResponseRedirects('/login');
    }

    private function registerFreshPlayer(string $email): Account
    {
        $crawler = $this->client->request('GET', '/join/join-peak-performance');
        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Fresh',
            'player_registration[accountLastName]' => 'Verifier',
            'player_registration[email]' => $email,
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Fresh',
            'player_registration[playerDateOfBirth]' => sprintf('%d-01-01', ((int) date('Y')) - 22),
        ]);
        $this->client->submit($form);

        return $this->account($email);
    }

    private function issueRawToken(string $forNewPlayerEmail): string
    {
        $this->registerFreshPlayer($forNewPlayerEmail);
        $account = $this->account($forNewPlayerEmail);

        /** @var SecureTokenFactory $tokenFactory */
        $tokenFactory = self::getContainer()->get(SecureTokenFactory::class);
        $generated = $tokenFactory->generate();

        /** @var EmailVerificationTokenRepository $tokens */
        $tokens = self::getContainer()->get(EmailVerificationTokenRepository::class);
        $tokens->add(new EmailVerificationToken($account, $generated->hash));
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->flush();

        return $generated->raw;
    }

    private function findQueuedEmailContaining(string $needle): ?Email
    {
        foreach (self::getMailerMessages() as $message) {
            if ($message instanceof Email && str_contains((string) $message->getTextBody(), $needle)) {
                return $message;
            }
        }

        return null;
    }

    private function extractToken(string $body, string $pathPrefix): string
    {
        $pattern = '#'.preg_quote($pathPrefix, '#').'(\S+)#';
        self::assertMatchesRegularExpression($pattern, $body, 'The email body should contain a link with a token.');
        preg_match($pattern, $body, $matches);

        return $matches[1];
    }
}
