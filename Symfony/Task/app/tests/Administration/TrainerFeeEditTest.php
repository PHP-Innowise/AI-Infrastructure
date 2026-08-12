<?php

declare(strict_types=1);

namespace App\Tests\Administration;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-05.10 — Super Admin Configures Per-Trainer Fee.
 */
final class TrainerFeeEditTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-05-27: Super Admin views and edits one trainer's monthly
     * subscription price and application fee rate from "Edit Pricing";
     * the new fee rate is read by every subsequent charge immediately
     * (FeeCalculator/StripeGateway::applyCurrentFee() always read the
     * current row, never a snapshot from before this edit — proven here
     * directly against the persisted settings row, not just the form
     * response).
     */
    public function testSuperAdminEditsATrainersSubscriptionPriceAndFeeRate(): void
    {
        $trainer = $this->trainer('peak-performance');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '500', 'AC-05-27: the current 5% (500bp) default fee rate is shown.');

        $form = $crawler->selectButton('Save')->form([
            'trainer_fee[monthlySubscriptionPriceMinorUnits]' => '2500',
            'trainer_fee[platformFeeBasisPoints]' => '800',
            'trainer_fee[reason]' => 'Early-adopter partner pricing.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);
        self::assertSame(2500, $settings->getMonthlySubscriptionPriceMinorUnits(), 'AC-05-27: the new subscription price is saved.');
        self::assertSame(800, $settings->getPlatformFeeBasisPoints(), 'AC-05-27: the new fee rate applies to new transactions immediately (read live, no snapshot).');
    }

    /**
     * AC-05-28: every per-trainer pricing change is audit-logged — who,
     * when, the old rate, the new rate, and the optional reason.
     *
     * The "old" values are pinned by a REAL prior edit through this same
     * HTTP form, not assumed to be Billing's own factory defaults
     * (1500/500) and not written directly through the entity manager —
     * this suite's fixture trainers are shared, mutable state across the
     * whole `make test` run (no per-test database reset, per the
     * Makefile's own `test-db` target), and both fixture trainers are
     * touched by other Billing tests that legitimately change pricing
     * (StripeConnectTest's own AC-05-2 default check is the other side of
     * this same coin: it relies on 'baseline-athletics' staying untouched,
     * which is exactly why this test uses 'peak-performance' and pins its
     * own starting values instead of trusting whatever they happen to be
     * when this test runs). A direct entity-manager write immediately
     * before the assertion request was tried first and found unreliable —
     * KernelBrowser reboots the kernel (and so the EntityManager/connection)
     * before each request by default, and this specific controller's
     * `AdministrativeScope::openFor()` cross-tenant path did not reliably
     * observe a write made just before the request on the pre-reboot
     * container. Two real requests avoid the question entirely.
     */
    public function testFeeEditIsAuditLoggedWithOldAndNewRatesAndReason(): void
    {
        $trainer = $this->trainer('peak-performance');
        $admin = $this->account('admin@practiceperfect.test');

        $this->client->loginUser($admin);

        // Pins the "old" state via a real edit through this same form,
        // exactly as testSuperAdminEditsATrainersSubscriptionPriceAndFeeRate()
        // above already proves works end to end.
        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));
        $pinForm = $crawler->selectButton('Save')->form([
            'trainer_fee[monthlySubscriptionPriceMinorUnits]' => '7700',
            'trainer_fee[platformFeeBasisPoints]' => '700',
            'trainer_fee[reason]' => 'Pinning a known starting rate for this test.',
        ]);
        $this->client->submit($pinForm);
        self::assertResponseRedirects();

        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));
        $form = $crawler->selectButton('Save')->form([
            'trainer_fee[monthlySubscriptionPriceMinorUnits]' => '1200',
            'trainer_fee[platformFeeBasisPoints]' => '300',
            'trainer_fee[reason]' => 'Promotional rate for Q1.',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->search('trainer.fee_edit', (int) $trainer->getId());
        self::assertNotEmpty($entries, 'AC-05-28: the fee edit is audit-logged.');

        // Matched on the SECOND edit's own known new values, not
        // "entries[0]" / search()'s own DESC-by-occurredAt ordering —
        // `audit_log_entry.occurred_at` is second-precision, and the pin
        // edit and the real edit above both run well within the same
        // wall-clock second, so their timestamps can tie and leave the
        // DESC ordering's tie-break undefined (the exact same class of
        // issue TokenPurchaseTest's own comments document for
        // `payment_record.created_at`).
        $entry = current(array_filter(
            $entries,
            static fn ($e) => 1200 === ($e->getDetails()['newMonthlySubscriptionPriceMinorUnits'] ?? null),
        ));
        self::assertNotFalse($entry, 'The real (second) edit\'s own audit entry is found.');
        self::assertNotNull($entry->getActorAccount());
        self::assertSame($admin->getId(), $entry->getActorAccount()->getId(), 'AC-05-28: who changed it.');
        // AC-05-28 "when": getOccurredAt() is a non-nullable
        // \DateTimeImmutable by its own type — every entry inherently
        // carries a timestamp, nothing further to assert here.

        $details = $entry->getDetails();
        self::assertSame(7700, $details['oldMonthlySubscriptionPriceMinorUnits'] ?? null, 'AC-05-28: the old subscription price.');
        self::assertSame(1200, $details['newMonthlySubscriptionPriceMinorUnits'] ?? null, 'AC-05-28: the new subscription price.');
        self::assertSame(700, $details['oldPlatformFeeBasisPoints'] ?? null, 'AC-05-28: the old fee rate.');
        self::assertSame(300, $details['newPlatformFeeBasisPoints'] ?? null, 'AC-05-28: the new fee rate.');
        self::assertSame('Promotional rate for Q1.', $details['reason'] ?? null, 'AC-05-28: the optional reason.');
    }

    /**
     * AC-05-28: the reason is genuinely optional.
     */
    public function testFeeEditReasonIsOptional(): void
    {
        $trainer = $this->trainer('peak-performance');

        $this->client->loginUser($this->account('admin@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));
        $form = $crawler->selectButton('Save')->form([
            'trainer_fee[monthlySubscriptionPriceMinorUnits]' => '1500',
            'trainer_fee[platformFeeBasisPoints]' => '500',
            'trainer_fee[reason]' => '',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects(message: 'AC-05-28: a blank reason does not block the edit.');
    }

    /**
     * A trainer (not Super Admin) cannot reach this tool at all.
     */
    public function testATrainerCannotEditFees(): void
    {
        $trainer = $this->trainer('peak-performance');

        $this->client->loginUser($trainer->getOwnerAccount());
        $this->client->request('GET', sprintf('/super-admin/trainers/%d/fees', $trainer->getId()));

        self::assertResponseStatusCodeSame(403);
    }
}
