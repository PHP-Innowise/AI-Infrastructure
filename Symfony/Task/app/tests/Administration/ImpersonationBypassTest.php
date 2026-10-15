<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Platform\Repository\ImpersonationSessionRepository;
use App\Tests\Support\FixtureHelpers;
use Doctrine\DBAL\Connection;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * BR-01-21/AC-01-37 and AC-01-36/76, asserted against the mechanism rather
 * than against the button.
 *
 * `ImpersonationTest` proves the voter refuses another Super Admin and that
 * the controller returns 403 — both true, both passing, and both irrelevant
 * to how impersonation actually happens. Symfony swaps identities on a query
 * parameter that works on every URL in the application, and manual testing
 * found that path went around all of it: `?_switch_user=<email>` impersonated
 * a second Super Admin, and impersonating an ordinary user this way left the
 * audit log with no record that anyone had done it.
 *
 * These tests never touch the Users tool. They append the parameter, which is
 * what an actual bypass looks like.
 */
final class ImpersonationBypassTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * BR-01-21/AC-01-37. The rule the epic states outright, on the path that
     * used to ignore it.
     */
    public function testTheQueryParameterCannotImpersonateAnotherSuperAdmin(): void
    {
        // Unique per run: the suite shares one mutable database, and a fixed
        // address makes a second filtered run fail on the unique index
        // rather than on the thing under test.
        $secondAdmin = $this->createSecondSuperAdmin(sprintf('bypass-second-admin-%s@practiceperfect.test', uniqid()));
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', '/dashboard?_switch_user='.$secondAdmin->getEmail());

        self::assertResponseStatusCodeSame(403, 'AC-01-37 must hold wherever the swap is requested, not only in the Users tool.');
        self::assertSame(0, $this->impersonationCountFor($secondAdmin), 'A refused swap records no session.');
    }

    /**
     * The voter also refuses an admin targeting themselves. Worth its own
     * case because it is the check most likely to be written as an object
     * comparison, and the firewall loads the target through the user
     * provider — not necessarily the same instance the token holds.
     */
    public function testTheQueryParameterCannotImpersonateYourself(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($admin);

        $this->client->request('GET', '/dashboard?_switch_user='.$admin->getEmail());

        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-01-36/76: a permitted impersonation started this way is recorded
     * exactly like one started from the Users tool — because both are now
     * recorded by the swap itself.
     */
    public function testAPermittedSwapIsStillFullyAudited(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('player@practiceperfect.test');

        $startsBefore = $this->auditCount('impersonation_start');
        $endsBefore = $this->auditCount('impersonation_end');

        $this->client->loginUser($admin);
        $this->client->request('GET', '/dashboard?_switch_user='.$target->getEmail());
        $this->client->followRedirect();

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('.impersonation-banner', $target->getEmail(), 'The swap did happen.');
        self::assertSame($startsBefore + 1, $this->auditCount('impersonation_start'), 'AC-01-76: no impersonation is silent.');

        /** @var ImpersonationSessionRepository $sessions */
        $sessions = self::getContainer()->get(ImpersonationSessionRepository::class);
        $opened = $sessions->findOpenSessionForAdmin($admin);
        self::assertNotNull($opened, 'AC-01-36: the session row exists for the history report.');
        $openedId = $opened->getId();

        // And the other end, through the same mechanism.
        $this->client->request('GET', '/dashboard?_switch_user=_exit');
        $this->client->followRedirect();

        self::assertSelectorNotExists('.impersonation-banner');
        self::assertSame($endsBefore + 1, $this->auditCount('impersonation_end'));

        // This session specifically, rather than "no open session for this
        // admin": the suite shares one database, so an unrelated leftover
        // would otherwise decide the result.
        self::assertNotNull($this->sessionEndedAt($openedId), 'The session this test opened is closed, not left open.');
    }

    /**
     * The pre-existing half of the gate, pinned so it cannot be lost while
     * rewriting the other half: the parameter is Super-Admin-only. A trainer
     * never had this capability and still does not.
     */
    public function testANonSuperAdminStillCannotUseTheQueryParameterAtAll(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $this->client->request('GET', '/dashboard?_switch_user=player@practiceperfect.test');

        self::assertResponseStatusCodeSame(403);
    }

    private function createSecondSuperAdmin(string $email): Account
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);

        $admin = new Account($email, '', AccountRole::SuperAdmin);
        $admin->changePasswordHash($hasher->hashPassword($admin, 'password'));
        $entityManager->persist($admin);
        $entityManager->persist(new AccountProfile($admin, 'Bypass', 'Admin'));
        $entityManager->flush();

        return $admin;
    }

    /**
     * Counted through DBAL: audit rows are written by a subscriber during a
     * request the test client has already finished, so a repository bound to
     * a pre-request container would read a stale identity map.
     */
    private function auditCount(string $actionType): int
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        return (int) $connection->fetchOne('SELECT count(*) FROM audit_log_entry WHERE action_type = ?', [$actionType]);
    }

    private function sessionEndedAt(?int $sessionId): ?string
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');
        $endedAt = $connection->fetchOne('SELECT ended_at FROM impersonation_session WHERE id = ?', [$sessionId]);

        return \is_string($endedAt) ? $endedAt : null;
    }

    private function impersonationCountFor(Account $target): int
    {
        /** @var Connection $connection */
        $connection = self::getContainer()->get('doctrine.dbal.default_connection');

        return (int) $connection->fetchOne('SELECT count(*) FROM impersonation_session WHERE target_account_id = ?', [$target->getId()]);
    }
}
