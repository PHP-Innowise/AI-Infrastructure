<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Repository\PasswordResetTokenRepository;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * AC-01-66: the forgot/reset-password flow works end to end.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-66, BR-01-4
 */
final class PasswordResetTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-66: request -> email -> confirm -> sign in with the new
     * password. Every step goes through the real HTTP forms, including the
     * final login, which is the only way to actually prove the password
     * changed rather than merely that the request "succeeded".
     */
    public function testPasswordResetWorksEndToEnd(): void
    {
        $crawler = $this->client->request('GET', '/password/forgot');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Send reset link')->form([
            'request_password_reset[email]' => 'trainer@practiceperfect.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/login');

        self::assertQueuedEmailCount(1);
        $email = self::getMailerMessage(0);
        self::assertInstanceOf(Email::class, $email);
        self::assertEmailAddressContains($email, 'To', 'trainer@practiceperfect.test');

        $token = $this->extractToken((string) $email->getTextBody(), '/password/reset/');

        $resetCrawler = $this->client->request('GET', '/password/reset/'.$token);
        self::assertResponseIsSuccessful();

        $resetForm = $resetCrawler->selectButton('Set password')->form([
            'set_new_password[plainPassword][first]' => 'a-brand-new-password',
            'set_new_password[plainPassword][second]' => 'a-brand-new-password',
        ]);
        $this->client->submit($resetForm);

        self::assertResponseRedirects('/login');
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'reset');

        // The real proof: sign in with the NEW password through the actual
        // form_login path.
        $loginCrawler = $this->client->request('GET', '/login');
        $loginForm = $loginCrawler->selectButton('Sign in')->form([
            '_username' => 'trainer@practiceperfect.test',
            '_password' => 'a-brand-new-password',
        ]);
        $this->client->submit($loginForm);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'trainer@practiceperfect.test');
    }

    /**
     * Enumeration-safe: requesting a reset for an email with no account
     * produces the identical outward response (redirect + flash) and queues
     * no email, so a visitor cannot use this form to test which addresses
     * are registered.
     */
    public function testRequestingAResetForAnUnknownEmailGivesTheSameOutwardResponse(): void
    {
        $crawler = $this->client->request('GET', '/password/forgot');
        $form = $crawler->selectButton('Send reset link')->form([
            'request_password_reset[email]' => 'nobody-registered@example.test',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/login');
        $this->client->followRedirect();
        self::assertSelectorTextContains('.flash--success', 'reset link is on its way');

        self::assertQueuedEmailCount(0, 'No email exists for this address, so nothing should be queued.');
    }

    /**
     * BR-01-4: a token is single-use. Resetting once and then replaying the
     * same link must not silently allow a second password change.
     *
     * Both attempts go through the real POST submission, not a bare GET:
     * PasswordController::reset() only calls PasswordResetService::consume()
     * once the form is actually submitted — a GET always renders the form
     * regardless of the token's state (unlike email verification, which
     * checks on GET). A bare-GET replay check would therefore pass for the
     * wrong reason.
     */
    public function testATokenCannotBeReplayedAfterUse(): void
    {
        /** @var \App\Platform\Service\SecureTokenFactory $tokenFactory */
        $tokenFactory = self::getContainer()->get(\App\Platform\Service\SecureTokenFactory::class);
        $account = $this->account('coach@practiceperfect.test');
        /** @var PasswordResetTokenRepository $tokens */
        $tokens = self::getContainer()->get(PasswordResetTokenRepository::class);
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        $generated = $tokenFactory->generate();
        $tokens->add(new \App\Identity\Entity\PasswordResetToken($account, $generated->hash));
        $em->flush();

        $firstCrawler = $this->client->request('GET', '/password/reset/'.$generated->raw);
        $firstForm = $firstCrawler->selectButton('Set password')->form([
            'set_new_password[plainPassword][first]' => 'first-new-password',
            'set_new_password[plainPassword][second]' => 'first-new-password',
        ]);
        $this->client->submit($firstForm);
        self::assertResponseRedirects('/login');

        // Replay: the identical link, submitted a second time.
        $replayCrawler = $this->client->request('GET', '/password/reset/'.$generated->raw);
        $replayForm = $replayCrawler->selectButton('Set password')->form([
            'set_new_password[plainPassword][first]' => 'second-new-password',
            'set_new_password[plainPassword][second]' => 'second-new-password',
        ]);
        $this->client->submit($replayForm);

        self::assertResponseStatusCodeSame(404);
        self::assertSelectorTextContains('h1', 'invalid or has expired');
    }

    /**
     * BR-01-4: 1-hour expiry. An old link fails the same way a replayed one
     * does — a generic "not usable" outcome, never a distinction between
     * "expired" and "already used" (that distinction would be an oracle).
     * Submitted, for the same reason as the replay test above.
     */
    public function testAnExpiredTokenIsRejected(): void
    {
        $account = $this->account('coach@practiceperfect.test');
        /** @var \App\Platform\Service\SecureTokenFactory $tokenFactory */
        $tokenFactory = self::getContainer()->get(\App\Platform\Service\SecureTokenFactory::class);
        $generated = $tokenFactory->generate();

        $token = new \App\Identity\Entity\PasswordResetToken($account, $generated->hash);
        /** @var PasswordResetTokenRepository $tokens */
        $tokens = self::getContainer()->get(PasswordResetTokenRepository::class);
        $tokens->add($token);
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->flush();

        $reflection = new \ReflectionProperty($token, 'expiresAt');
        $reflection->setAccessible(true);
        $reflection->setValue($token, new \DateTimeImmutable('-1 minute'));
        $em->flush();

        $crawler = $this->client->request('GET', '/password/reset/'.$generated->raw);
        $form = $crawler->selectButton('Set password')->form([
            'set_new_password[plainPassword][first]' => 'irrelevant-new-password',
            'set_new_password[plainPassword][second]' => 'irrelevant-new-password',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(404);
    }

    private function extractToken(string $body, string $pathPrefix): string
    {
        $pattern = '#'.preg_quote($pathPrefix, '#').'(\S+)#';
        self::assertMatchesRegularExpression($pattern, $body, 'The email body should contain a link with a token.');
        preg_match($pattern, $body, $matches);

        return $matches[1];
    }
}
