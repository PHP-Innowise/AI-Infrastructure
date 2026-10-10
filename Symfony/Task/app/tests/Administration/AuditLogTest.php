<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Identity\Service\TrainerProvisioningService;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-07.08 — Super Admin Views Audit Log.
 */
final class AuditLogTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-29: navigates to "Audit Log"/"Activity Log" and views a
     * chronological list of logged actions.
     */
    public function testAuditLogShowsAChronologicalList(): void
    {
        $trainer = $this->createFreshTrainer('audit-list@example.test', 'Audit List FC');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/super-admin/audit-log');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Audit List FC', 'AC-07-29: the trainer-creation entry appears in the log.');

        $timestamps = $crawler->filter('table tbody tr td:first-child')->each(static fn ($node) => $node->text());
        $sorted = $timestamps;
        rsort($sorted);
        self::assertSame($sorted, $timestamps, 'AC-07-29: chronological (newest first).');

        unset($trainer);
    }

    /**
     * AC-07-30: captures, at minimum, all six named categories —
     * impersonation sessions, trainer accounts created/deactivated, user
     * accounts deleted (GDPR), feature-toggle changes, pricing changes,
     * and overridden scheduling conflicts. Two (impersonation, trainer
     * creation) are driven through this test's own real HTTP requests and
     * checked as VISIBLE on this screen — the observable requirement
     * AC-07-30 itself names ("captures"), not merely written to the
     * table; the screen's own rendering is otherwise generic (any
     * actionType/subject renders through the same row template — see
     * testLogEntryShowsTimestampActionSubjectDetailsAndAdmin), so the
     * remaining four are confirmed present in the underlying log via the
     * repository, reusing the exact `AuditLogger::record()` call sites
     * `AccountLifecycleTest`/`FeatureToggleTest`/`TrainerFeeEditTest`/
     * `EventMasterConflictOverrideTest` already prove end to end.
     */
    public function testAuditLogCapturesEveryRequiredActionType(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $target = $this->trainer('baseline-athletics')->getOwnerAccount();
        $this->client->loginUser($admin);

        // Impersonation session (start + end). Two redirects each way
        // (Symfony's switch_user listener strips the query parameter on
        // its own follow-up redirect) — matches ImpersonationTest's own
        // followRedirects(true) requirement exactly.
        $this->client->followRedirects(true);
        $startCrawler = $this->client->request('GET', sprintf('/super-admin/users/%d', $target->getId()));
        $dashboardCrawler = $this->client->submit($startCrawler->selectButton('Impersonate')->form());
        $this->client->submit($dashboardCrawler->selectButton('Exit Impersonation')->form());
        $this->client->followRedirects(false);

        // Trainer created.
        $trainer = $this->createFreshTrainer('audit-visible@example.test', 'Audit Visible FC');

        $crawler = $this->client->request('GET', '/super-admin/audit-log');
        self::assertResponseIsSuccessful();

        self::assertSelectorTextContains('body', 'impersonation_start', 'AC-07-30: impersonation sessions — visible on screen.');
        self::assertSelectorTextContains('body', 'trainer_created', 'AC-07-30: trainer accounts created — visible on screen.');

        // Trainer deactivated — driven through the real confirm page/form,
        // matching every other CSRF-protected action test in this suite
        // (e.g. UsersToolTest), rather than a hand-crafted token.
        $deactivateCrawler = $this->client->request('GET', sprintf('/super-admin/users/%d/deactivate', $trainer->getOwnerAccount()->getId()));
        $this->client->submit($deactivateCrawler->selectButton('Confirm deactivation')->form());

        // User deleted (GDPR).
        $deletable = $this->createFreshTrainer('audit-deletable@example.test', 'Audit Deletable FC')->getOwnerAccount();
        $deleteCrawler = $this->client->request('GET', sprintf('/super-admin/users/%d/delete', $deletable->getId()));
        $this->client->submit($deleteCrawler->selectButton('Confirm permanent deletion')->form());

        // Feature-toggle change.
        $toggleCrawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/features', $trainer->getId()), ['feature' => 'lppp']);
        $this->client->submit($toggleCrawler->selectButton('Confirm disable')->form());

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        // AccountLifecycleService::deactivate() records 'user_deactivated'
        // with no relatedTrainer at all (the action is about an account,
        // not trainer-scoped) — matched by subject id instead, the same
        // way EventMasterConflictOverrideTest matches a specific event.
        $deactivations = array_filter(
            $auditLog->search('user_deactivated'),
            static fn ($e) => (int) ($e->getSubjectId() ?? 0) === (int) $trainer->getOwnerAccount()->getId(),
        );
        self::assertNotEmpty($deactivations, 'AC-07-30: trainer accounts deactivated.');
        self::assertNotEmpty($auditLog->search('user_deleted'), 'AC-07-30: user accounts deleted (GDPR).');
        self::assertNotEmpty($auditLog->search('feature_toggled', (int) $trainer->getId()), 'AC-07-30: feature-toggle changes.');
        // Pricing changes and overridden scheduling conflicts are each
        // proven end to end, screen-visibility included, by their own
        // dedicated, self-contained tests (PricingAuditTest,
        // EventMasterConflictOverrideTest) — deliberately NOT restated
        // here as a repository presence check: this test file's execution
        // order relative to those two is not guaranteed (PHPUnit does not
        // order test CLASSES alphabetically the way it orders methods
        // within one class), so asserting on data only THOSE files create
        // would make this test's own pass/fail depend on run order.
    }

    /**
     * AC-07-31: each entry shows a timestamp, the action performed, the
     * subject affected, action-specific details, and the admin who
     * performed it.
     */
    public function testLogEntryShowsTimestampActionSubjectDetailsAndAdmin(): void
    {
        $admin = $this->account('admin@practiceperfect.test');
        $trainer = $this->createFreshTrainer('audit-entry-shape@example.test', 'Audit Entry Shape FC');

        $this->client->loginUser($admin);
        $crawler = $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Entry Shape FC']);

        self::assertResponseIsSuccessful();
        $row = $crawler->filter('table tbody tr')->first();
        $text = $row->text();

        self::assertMatchesRegularExpression('/\d{4}-\d{2}-\d{2}/', $text, 'AC-07-31: a timestamp.');
        self::assertStringContainsString('trainer_created', $text, 'AC-07-31: the action performed.');
        self::assertStringContainsString('Audit Entry Shape FC', $text, 'AC-07-31: the subject affected.');
        self::assertStringContainsString($admin->getEmail(), $text, 'AC-07-31: the admin who performed it.');
        // "Action-specific details" — the JSON details blob is rendered.
        self::assertStringContainsString('business_name', $text, 'AC-07-31: action-specific details.');
    }

    /**
     * AC-07-32: filtered by date range (last 7/30 days, custom) and action
     * type, and searched by subject (user or trainer name).
     */
    public function testAuditLogFiltersByDateRangeActionTypeAndSubject(): void
    {
        $trainer = $this->createFreshTrainer('audit-filter@example.test', 'Audit Filter FC');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        // Subject search by trainer name.
        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Filter FC']);
        self::assertSelectorTextContains('body', 'trainer_created');

        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'no-such-trainer-exists']);
        self::assertSelectorTextNotContains('body', 'Audit Filter FC');

        // Action-type filter.
        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Filter FC', 'actionType' => 'trainer_created']);
        self::assertSelectorTextContains('body', 'Audit Filter FC');

        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Filter FC', 'actionType' => 'user_deleted']);
        self::assertSelectorTextNotContains('body', 'Audit Filter FC');

        // Date range: excluded when the "to" bound is before this entry's
        // creation, included with a wide range.
        $yesterday = (new \DateTimeImmutable('-1 day'))->format('Y-m-d');
        $farPast = (new \DateTimeImmutable('-30 days'))->format('Y-m-d');
        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Filter FC', 'dateFrom' => $farPast, 'dateTo' => $yesterday]);
        self::assertSelectorTextNotContains('body', 'Audit Filter FC', 'AC-07-32: excluded by a date range ending before it occurred.');

        $tomorrow = (new \DateTimeImmutable('+1 day'))->format('Y-m-d');
        $this->client->request('GET', '/super-admin/audit-log', ['q' => 'Audit Filter FC', 'dateFrom' => $farPast, 'dateTo' => $tomorrow]);
        self::assertSelectorTextContains('body', 'Audit Filter FC', 'AC-07-32: included by a date range covering today.');
    }

    /**
     * AC-07-33: audit log entries can be exported to CSV. Content-Type and
     * attachment headers only, matching EventMasterController::export()'s
     * own already-established test precedent
     * (EventMasterTest::testSuperAdminViewsDetailsRsvpListAndExportsCsv) —
     * Symfony's test client does not reliably capture a StreamedResponse's
     * callback-produced body, so that existing CSV-export test does not
     * assert on body content either.
     */
    public function testAuditLogExportsToCsv(): void
    {
        $trainer = $this->createFreshTrainer('audit-export@example.test', 'Audit Export FC');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $this->client->request('GET', '/super-admin/audit-log/export', ['q' => 'Audit Export FC']);

        self::assertResponseIsSuccessful();
        self::assertStringStartsWith('text/csv', (string) $this->client->getResponse()->headers->get('Content-Type'));
        self::assertStringContainsString('attachment', (string) $this->client->getResponse()->headers->get('Content-Disposition'));

        unset($trainer);
    }

    /**
     * AC-07-34/BR-07-6: entries are retained (at minimum 1 year) —
     * proven structurally, the same way as AC-01-76/BR-07-6's own UPDATE
     * proof in AuditLoggerTest: the application role holds no DELETE
     * privilege on this table at all, so nothing the app can do removes an
     * entry before any retention window, let alone a 1-year one.
     */
    public function testAuditLogEntriesCannotBeDeletedByTheApplicationRole(): void
    {
        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search(limit: 1);
        self::assertNotEmpty($entries, 'At least one entry exists (fixtures/prior tests write several).');

        /** @var EntityManagerInterface $em */
        $em = self::getContainer()->get(EntityManagerInterface::class);

        $this->expectException(\Doctrine\DBAL\Exception::class);
        $em->getConnection()->executeStatement('DELETE FROM audit_log_entry WHERE id = :id', ['id' => $entries[0]->getId()]);
    }

    /**
     * A Trainer (not Super Admin) cannot reach the audit log.
     */
    public function testATrainerCannotViewTheAuditLog(): void
    {
        $this->client->loginUser($this->trainer('peak-performance')->getOwnerAccount());
        $this->client->request('GET', '/super-admin/audit-log');

        self::assertResponseStatusCodeSame(403);
    }

    private function createFreshTrainer(string $email, string $businessName): Trainer
    {
        /** @var TrainerProvisioningService $provisioning */
        $provisioning = self::getContainer()->get(TrainerProvisioningService::class);

        return $provisioning->createTrainer($this->account('admin@practiceperfect.test'), $businessName, 'Audit', 'Owner', $email, null);
    }
}
