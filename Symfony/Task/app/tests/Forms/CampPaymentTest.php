<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Repository\PaymentRecordRepository;
use App\Forms\Entity\FormSubmission;
use App\Forms\Repository\FormSubmissionRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use App\Tests\Support\WebhookDeliveryHelper;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-08.03 (paid path): BR-08-8/11/12/14 — Stripe Checkout only, payment
 * completes before confirmation, the platform fee applies, and the async
 * webhook is what actually confirms.
 */
final class CampPaymentTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;
    use BillingFixtureHelpers;
    use WebhookDeliveryHelper;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-08-15 (paid path)/BR-08-8: redirects to Stripe Checkout; a
     * pending payment does not count as a registration.
     */
    public function testPaidCampRedirectsToCheckoutAndStaysPendingUntilConfirmed(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Paid Camp Redirect', 'priceMinorUnits' => 5000]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Payer Participant',
            'form_submission[participant_email]' => 'payer@example.test',
        ]);
        $this->client->submit($submissionForm);

        self::assertResponseRedirects();
        self::assertStringContainsString('checkout.stripe.test', (string) $this->client->getResponse()->headers->get('Location'));

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'payer@example.test');
        self::assertNotNull($submission);
        self::assertSame(FormSubmission::STATUS_PENDING, $submission->getPaymentStatus());
        self::assertFalse($submission->isConfirmed(), 'BR-08-8: pending payment does not count as a registration.');

        // AC-08-15: does not count toward capacity.
        self::assertSame(0, $submissions->countConfirmedForForm($form));
    }

    /**
     * BR-08-14: the Stripe webhook confirms payment, which marks the
     * registration Paid, and only then sends the confirmation email
     * (AC-08-16).
     */
    public function testWebhookConfirmsPaymentAndSendsConfirmation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Webhook Camp', 'priceMinorUnits' => 4000]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Confirmed Payer',
            'form_submission[participant_email]' => 'confirmed-payer@example.test',
        ]);
        $this->client->submit($submissionForm);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'confirmed-payer@example.test');
        self::assertNotNull($submission);
        $paymentRecord = $submission->getPaymentRecord();
        self::assertNotNull($paymentRecord);
        self::assertNotNull($paymentRecord->getStripePaymentIntentId());

        $this->deliverPaymentIntentSucceeded($paymentRecord->getStripePaymentIntentId());

        $this->activateTenant($trainer);
        $paidSubmission = $submissions->findOneByFormAndEmail($form, 'confirmed-payer@example.test');
        self::assertNotNull($paidSubmission);
        self::assertSame(FormSubmission::STATUS_PAID, $paidSubmission->getPaymentStatus());
        self::assertTrue($paidSubmission->isConfirmed());
        self::assertSame(1, $submissions->countConfirmedForForm($form));

        self::assertQueuedEmailCount(1);
        $message = self::getMailerMessages()[0];
        self::assertInstanceOf(Email::class, $message);
        self::assertSame(['confirmed-payer@example.test'], array_map(static fn ($a) => $a->getAddress(), $message->getTo()));
    }

    /**
     * BR-08-12: the platform's 5% fee is applied to camp revenue.
     */
    public function testPlatformFeeAppliesToCampRevenue(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Fee Camp', 'priceMinorUnits' => 10000]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Fee Payer',
            'form_submission[participant_email]' => 'fee-payer@example.test',
        ]);
        $this->client->submit($submissionForm);

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'fee-payer@example.test');
        self::assertNotNull($submission);
        $paymentRecord = $submission->getPaymentRecord();
        self::assertNotNull($paymentRecord);

        // $100.00 at 5% (500 bps, the platform default) = $5.00 exactly —
        // database-designer-schema.md "Money and the platform fee",
        // verified against this exact worked example.
        self::assertSame(500, $paymentRecord->getFeeRateBasisPoints());
        self::assertSame(500, $paymentRecord->getPlatformFeeMinorUnits());
    }

    /**
     * A3/A4: a camp registrant has no account; the payment is recorded
     * platform-side against the submission, not a player account.
     */
    public function testCampPaymentRecordHasNoPayerAccount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);
        $form = $this->createCamp($trainer, ['name' => 'No Payer Account Camp', 'priceMinorUnits' => 2500]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'No Account Payer',
            'form_submission[participant_email]' => 'no-account-payer@example.test',
        ]);
        $this->client->submit($submissionForm);

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'no-account-payer@example.test');
        self::assertNotNull($submission);
        $paymentRecord = $submission->getPaymentRecord();
        self::assertNotNull($paymentRecord);

        self::assertNull($paymentRecord->getPayerAccount(), 'A3/A4: a camp payer may have no account.');
        self::assertSame(PaymentRecord::TYPE_CAMP_REGISTRATION, $paymentRecord->getType());
        self::assertSame(PaymentRecord::METHOD_CARD, $paymentRecord->getPaymentMethod(), 'BR-08-11: Stripe Checkout only, never token.');
        self::assertSame('no-account-payer@example.test', $paymentRecord->getContactEmail());
        self::assertNull($submission->getConvertedAccount());
    }
}
