<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * "In Scope (MVP)" § "System Configuration" — AC-07-40: pricing
 * configuration per trainer is Epic-05's own mechanism
 * (`administration_trainer_fee_edit`, already proven end to end by
 * `TrainerFeeEditTest::testFeeEditIsAuditLoggedWithOldAndNewRatesAndReason()`
 * under AC-05-28's own numbering); this epic's own addition is that the
 * change is visible on the Super Admin Audit Log screen specifically
 * ("Pricing changed (per-trainer rates)", § "US-07.08..." § "Logged
 * Actions"), which is what this test adds under its own AC-07-40/AC-07-32
 * citation.
 */
final class PricingAuditTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-07-40: pricing changes appear in the Super Admin audit log, and
     * AC-07-32 proves it is actually surfaced on the Audit Log screen
     * (not merely written to the table, which TrainerFeeEditTest already
     * covers under AC-05-28).
     */
    public function testPricingChangeAppearsOnTheAuditLogScreen(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('admin@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));
        $form = $crawler->selectButton('Save')->form([
            'trainer_fee[monthlySubscriptionPriceMinorUnits]' => '3300',
            'trainer_fee[platformFeeBasisPoints]' => '600',
            'trainer_fee[reason]' => 'AC-07-40 visibility check.',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $logCrawler = $this->client->request('GET', '/super-admin/audit-log', ['actionType' => 'trainer.fee_edit', 'q' => $trainer->getBusinessName()]);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'trainer.fee_edit', 'AC-07-40: the pricing change is visible on the Audit Log screen.');
        self::assertSelectorTextContains('body', $trainer->getBusinessName());

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('trainer.fee_edit', (int) $trainer->getId());
        self::assertNotEmpty($entries);
    }
}
