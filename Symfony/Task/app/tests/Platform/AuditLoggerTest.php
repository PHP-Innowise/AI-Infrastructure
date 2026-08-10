<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Service\AccountLifecycleService;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * AC-01-76: audit logs capture all sensitive operations, including
 * impersonation and user deletion.
 */
final class AuditLoggerTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
        $this->client->followRedirects(true);
    }

    /**
     * AC-01-76: impersonation start/end are captured.
     */
    public function testImpersonationIsAudited(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($admin);

        $crawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        $form = $crawler->selectButton('Impersonate')->form();
        $dashboardCrawler = $this->client->submit($form);
        $exitForm = $dashboardCrawler->selectButton('Exit Impersonation')->form();
        $this->client->submit($exitForm);

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $starts = $auditLog->search('impersonation_start');
        $ends = $auditLog->search('impersonation_end');

        self::assertNotEmpty($starts, 'AC-01-76: impersonation start is logged.');
        self::assertNotEmpty($ends, 'AC-01-76: impersonation end is logged.');
        self::assertSame($admin->getId(), $starts[0]->getActorAccount()?->getId(), 'The true actor is the Super Admin, not the target.');
    }

    /**
     * AC-01-76: GDPR deletion is captured.
     */
    public function testUserDeletionIsAudited(): void
    {
        $admin = $this->account('admin@practiceperfect.test');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);
        $target = new Account('audited-delete@example.test', '', AccountRole::Player);
        $target->changePasswordHash($hasher->hashPassword($target, 'password'));
        $em->persist($target);
        $em->persist(new AccountProfile($target, 'Audited', 'Delete'));
        $em->flush();

        /** @var AccountLifecycleService $service */
        $service = self::getContainer()->get(AccountLifecycleService::class);
        $service->anonymize($admin, $target, 'test reason');

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $deletions = $auditLog->search('user_deleted');

        self::assertNotEmpty($deletions, 'AC-01-76: user deletion is logged.');
    }

    /**
     * The append-only guarantee behind AC-01-76/BR-07-6: the application
     * role holds INSERT + SELECT only on audit_log_entry — enforced as a
     * database privilege (see the migration's REVOKE), not merely a code
     * convention that could be bypassed.
     */
    public function testAuditLogEntriesCannotBeUpdatedByTheApplicationRole(): void
    {
        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search(limit: 1);

        if ([] === $entries) {
            $admin = $this->account('admin@practiceperfect.test');
            $auditLog->add(new \App\Platform\Entity\AuditLogEntry($admin, 'test_event', 'account', $admin->getId(), null));
            /** @var EntityManagerInterface $em */
            $em = self::getContainer()->get(EntityManagerInterface::class);
            $em->flush();
            $entries = $auditLog->search(limit: 1);
        }

        self::assertNotEmpty($entries);

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);
        $this->expectException(\Doctrine\DBAL\Exception::class);
        $em->getConnection()->executeStatement('UPDATE audit_log_entry SET action_type = :a WHERE id = :id', [
            'a' => 'tampered',
            'id' => $entries[0]->getId(),
        ]);
    }
}
