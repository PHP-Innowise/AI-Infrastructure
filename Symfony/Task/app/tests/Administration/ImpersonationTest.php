<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Platform\Entity\ImpersonationSession;
use App\Platform\Repository\ImpersonationSessionRepository;
use App\Platform\Voter\ImpersonationVoter;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Crawler;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;
use Symfony\Component\Security\Core\Authorization\AuthorizationCheckerInterface;

/**
 * US-01.07 — Super Admin Impersonates User.
 *
 * Every successful `?_switch_user=` hit issues a SECOND redirect of its own
 * (Symfony's SwitchUserListener strips the query parameter once the token is
 * swapped), so `followRedirects(true)` is used throughout rather than a
 * single `followRedirect()` call.
 */
final class ImpersonationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
        $this->client->followRedirects(true);
    }

    /**
     * AC-01-33: impersonation is started via a confirmation-gated POST
     * naming the target.
     */
    public function testSuperAdminStartsImpersonationFromTheUsersTool(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Impersonate')->form();
        $this->client->submit($form);

        self::assertResponseIsSuccessful();
    }

    /**
     * AC-01-34: after confirming, the portal switches to the impersonated
     * user's view with a sticky banner; navigation/permissions/data match
     * the impersonated user exactly (here: reaching the trainer dashboard).
     */
    public function testImpersonationSwitchesToTheTargetsViewWithABanner(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $this->beginImpersonation($target);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('.impersonation-banner', 'Viewing as trainer@practiceperfect.test');
        self::assertSelectorTextContains('body', 'trainer@practiceperfect.test');

        // AC-01-34: reachability matches the target exactly — /super-admin/
        // becomes unreachable, since the effective role is now the target's.
        $this->client->request('GET', '/super-admin/users');
        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-01-35: exiting returns the Super Admin to their own view.
     */
    public function testExitingImpersonationReturnsToTheSuperAdmin(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $dashboardCrawler = $this->beginImpersonation($target);

        $exitForm = $dashboardCrawler->selectButton('Exit Impersonation')->form();
        $this->client->submit($exitForm);

        self::assertResponseIsSuccessful();
        self::assertSelectorNotExists('.impersonation-banner');
        // Back to being the Super Admin: /super-admin/ is reachable again.
        $this->client->request('GET', '/super-admin/users');
        self::assertResponseIsSuccessful();
    }

    /**
     * AC-01-36: every session is logged (who, whom, start, end, duration),
     * and an Impersonation History is available.
     */
    public function testImpersonationSessionIsLoggedWithStartEndAndDuration(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $dashboardCrawler = $this->beginImpersonation($target);
        $exitForm = $dashboardCrawler->selectButton('Exit Impersonation')->form();
        $this->client->submit($exitForm);

        /** @var ImpersonationSessionRepository $sessions */
        $sessions = self::getContainer()->get(ImpersonationSessionRepository::class);
        $history = $sessions->findAll();
        $match = current(array_filter(
            $history,
            static fn (ImpersonationSession $s) => $s->getTargetAccount()->getEmail() === 'trainer@practiceperfect.test',
        ));

        self::assertNotFalse($match, 'AC-01-36: the session is recorded (the Impersonation History report reads this table).');
        self::assertNotNull($match->getEndedAt(), 'AC-01-36: end time is logged.');
        self::assertNotNull($match->durationSeconds(), 'AC-01-36: duration is logged.');
        self::assertSame(ImpersonationSession::REASON_MANUAL, $match->getEndedReason());
    }

    /**
     * AC-01-37: a Super Admin cannot impersonate another Super Admin — a
     * validation error, not a silent no-op.
     */
    public function testSuperAdminCannotImpersonateAnotherSuperAdmin(): void
    {
        $admin = $this->account('admin@practiceperfect.test');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);
        $secondAdmin = new Account('second-admin@practiceperfect.test', '', AccountRole::SuperAdmin);
        $secondAdmin->changePasswordHash($hasher->hashPassword($secondAdmin, 'password'));
        $em->persist($secondAdmin);
        $em->persist(new AccountProfile($secondAdmin, 'Second', 'Admin'));
        $em->flush();

        // The Users tool's own "Impersonate" button never renders for a
        // Super Admin target at all (administration/user_show.html.twig),
        // so there is no genuine form/CSRF token to submit through HTTP for
        // this specific case — that hidden button is itself a first line of
        // defense, and the voter is the enforced one. Checked directly here,
        // exactly as ImpersonationController does via denyAccessUnlessGranted.
        $this->client->loginUser($admin);
        /** @var AuthorizationCheckerInterface $authChecker */
        $authChecker = self::getContainer()->get(AuthorizationCheckerInterface::class);

        self::assertFalse(
            $authChecker->isGranted(ImpersonationVoter::IMPERSONATION_START, $secondAdmin),
            'AC-01-37: blocked with a validation error, not silently allowed.',
        );

        // At the HTTP layer: denyAccessUnlessGranted() runs before the CSRF
        // check in ImpersonationController::start(), so even a request
        // carrying no valid token is refused for this exact reason (403),
        // not because of an incidental CSRF failure.
        $this->client->request('POST', sprintf('/super-admin/users/%d/impersonate', $secondAdmin->getId()));
        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-01-38: a session expires automatically after 1 hour if not
     * explicitly exited — enforced at read time, on the next request.
     */
    public function testImpersonationSessionExpiresAutomaticallyAfterOneHour(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $this->beginImpersonation($target);

        // Backdate the session's start time past the 1-hour limit.
        /** @var ImpersonationSessionRepository $sessions */
        $sessions = self::getContainer()->get(ImpersonationSessionRepository::class);
        $open = $sessions->findOpenSessionForAdmin($admin);
        self::assertNotNull($open);
        $sessionId = $open->getId();
        $reflection = new \ReflectionProperty($open, 'startedAt');
        $reflection->setAccessible(true);
        $reflection->setValue($open, new \DateTimeImmutable('-2 hours'));
        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $em->flush();

        // The next request must force an exit back to the admin's own view.
        $this->client->request('GET', '/dashboard');

        self::assertResponseIsSuccessful();
        self::assertSelectorNotExists('.impersonation-banner', 'AC-01-38: the session auto-expired.');

        // Re-fetch this SPECIFIC session by id, not "any open session for
        // this admin": other tests in this file deliberately leave their own
        // impersonation sessions open (to assert the still-active state),
        // and the /dashboard request above rebooted the kernel, so the
        // $sessions instance obtained before it is stale regardless.
        /** @var ImpersonationSessionRepository $freshSessions */
        $freshSessions = self::getContainer()->get(ImpersonationSessionRepository::class);
        $reloaded = $freshSessions->find($sessionId);
        self::assertNotNull($reloaded);
        self::assertNotNull($reloaded->getEndedAt(), 'AC-01-38: the session should now be closed.');
        self::assertSame(ImpersonationSession::REASON_EXPIRED, $reloaded->getEndedReason());
    }

    private function beginImpersonation(Account $target): Crawler
    {
        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        $form = $crawler->selectButton('Impersonate')->form();

        return $this->client->submit($form);
    }
}
