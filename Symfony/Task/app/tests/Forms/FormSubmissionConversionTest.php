<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Repository\PaymentRecordRepository;
use App\Forms\Entity\FormSubmission;
use App\Forms\Repository\FormSubmissionRepository;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Tests\Support\BillingFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use App\Tests\Support\WebhookDeliveryHelper;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-08.05: converting a camp/evaluation submission into a full account —
 * AC-08-23..26, BR-08-15..18, A5.
 */
final class FormSubmissionConversionTest extends WebTestCase
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
     * AC-08-23: "Create Your Account" opens a form pre-filled (read-only)
     * with the submission's name/email.
     */
    public function testConversionFormIsPrefilledWithSubmissionData(): void
    {
        $submission = $this->confirmedFreeSubmission('Casey Convertee', 'casey.convertee@example.test');

        $crawler = $this->client->request('GET', $this->convertAccountUrl($submission));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Casey Convertee');
        self::assertSelectorTextContains('body', 'casey.convertee@example.test');
        // Never rendered as editable inputs named after the account fields.
        self::assertCount(0, $crawler->filter('input[name="convert_submission_to_account[email]"]'));
    }

    /**
     * AC-08-24/BR-08-17: account created, auto-assigned to the trainer,
     * redirected to the player dashboard.
     */
    public function testConvertingCreatesAccountAssignsTrainerAndRedirects(): void
    {
        $trainer = $this->trainer('peak-performance');
        $submission = $this->confirmedFreeSubmission('Drew Adult', 'drew.adult@example.test');

        $crawler = $this->client->request('GET', $this->convertAccountUrl($submission));
        $form = $crawler->selectButton('Create Account')->form([
            'convert_submission_to_account[plainPassword][first]' => 'correct-horse-battery',
            'convert_submission_to_account[plainPassword][second]' => 'correct-horse-battery',
            'convert_submission_to_account[dateOfBirth]' => sprintf('%d-06-01', ((int) date('Y')) - 30),
            'convert_submission_to_account[acceptTerms]' => true,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        // account() itself already asserts the account exists (see
        // FixtureHelpers) -- this line is the real assertion: THIS is a
        // brand-new account created by the conversion just above.
        $account = $this->account('drew.adult@example.test');

        $this->activateTenant($trainer);
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        $player = $playerProfiles->findOneForSelfAccount($account);
        self::assertNotNull($player);
        self::assertSame('Drew Adult', $player->getFirstName());

        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($trainer, $player);
        self::assertNotNull($membership, 'BR-08-17: auto-assigned to the trainer.');
        self::assertTrue($membership->isActive());
        self::assertSame(PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION, $membership->getSource(), 'A3: the fourth CRM association source.');

        // The submission itself is marked converted.
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $reloaded = $submissions->find($submission->getId());
        self::assertNotNull($reloaded);
        self::assertTrue($reloaded->isConverted());
        self::assertSame($account->getId(), $reloaded->getConvertedAccount()?->getId());
    }

    /**
     * A1's child/adult branch applies identically here as it does to
     * ShareLink registration (PlayerRegistrationService::
     * registerViaCampConversion()'s own docblock): a submitter entering a
     * date of birth under 18 creates a PARENT-managed player, never a
     * self-account minor. The resulting account is the PARENT's own login;
     * the player profile is linked via ParentChildLink, not
     * self_account_id.
     */
    public function testConvertingWithAChildDateOfBirthCreatesAParentManagedPlayer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $submission = $this->confirmedFreeSubmission('Parent Convertee', 'parent.convertee@example.test');

        $crawler = $this->client->request('GET', $this->convertAccountUrl($submission));
        $form = $crawler->selectButton('Create Account')->form([
            'convert_submission_to_account[plainPassword][first]' => 'correct-horse-battery',
            'convert_submission_to_account[plainPassword][second]' => 'correct-horse-battery',
            // A 10-year-old today.
            'convert_submission_to_account[dateOfBirth]' => sprintf('%d-06-01', ((int) date('Y')) - 10),
            'convert_submission_to_account[acceptTerms]' => true,
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');

        $parentAccount = $this->account('parent.convertee@example.test');

        $this->activateTenant($trainer);
        /** @var PlayerProfileRepository $playerProfiles */
        $playerProfiles = self::getContainer()->get(PlayerProfileRepository::class);
        // A1: never a self-account for a minor.
        self::assertNull($playerProfiles->findOneForSelfAccount($parentAccount));

        /** @var ParentChildLinkRepository $parentChildLinks */
        $parentChildLinks = self::getContainer()->get(ParentChildLinkRepository::class);
        $links = $parentChildLinks->findByParent($parentAccount);
        self::assertCount(1, $links);
        $child = $links[0]->getChildPlayer();
        self::assertSame('Parent Convertee', $child->getFirstName());
        self::assertTrue($child->isChild());

        /** @var PlayerTrainerMembershipRepository $memberships */
        $memberships = self::getContainer()->get(PlayerTrainerMembershipRepository::class);
        $membership = $memberships->findOneByTrainerAndPlayer($trainer, $child);
        self::assertNotNull($membership, 'BR-08-17: the child is auto-assigned to the trainer, same as an adult.');
        self::assertSame(PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION, $membership->getSource());
    }

    /**
     * AC-08-25: declining conversion is never forced — the confirmation
     * email itself carries a ShareLink to register later (see
     * FormsMailer::sendConfirmation()'s own docblock for why this travels
     * in the one email actually sent, not a separate later one).
     */
    public function testConfirmationEmailOffersARegisterLaterLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Decline Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Undecided Participant',
            'form_submission[participant_email]' => 'undecided@example.test',
        ]);
        $this->client->submit($submissionForm);

        self::assertQueuedEmailCount(1);
        $message = self::getMailerMessages()[0];
        self::assertInstanceOf(Email::class, $message);
        self::assertStringContainsString('/join/', (string) $message->getTextBody(), 'AC-08-25: a ShareLink to register later.');
    }

    /**
     * AC-08-26/BR-08-18: the submission's email already has an account —
     * prompted to log in instead of creating a duplicate, and the
     * conversion form is never rendered.
     */
    public function testExistingAccountEmailPromptsLoginInstead(): void
    {
        // player@practiceperfect.test already exists in AppFixtures.
        $submission = $this->confirmedFreeSubmission('Existing Player', 'player@practiceperfect.test');

        $this->client->request('GET', $this->convertAccountUrl($submission));

        self::assertResponseRedirects('/login');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'already exists');
    }

    /**
     * A5: on conversion, the earlier camp payment and registration attach
     * to the new account — the existing PaymentRecord row is updated, not
     * duplicated.
     */
    public function testConversionAttachesEarlierPaymentToNewAccount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->connectStripe($trainer);
        $form = $this->createCamp($trainer, ['name' => 'A5 Camp', 'priceMinorUnits' => 3000]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'A5 Payer',
            'form_submission[participant_email]' => 'a5-payer@example.test',
        ]);
        $this->client->submit($submissionForm);

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'a5-payer@example.test');
        self::assertNotNull($submission);
        $paymentRecord = $submission->getPaymentRecord();
        self::assertNotNull($paymentRecord);
        $paymentRecordId = $paymentRecord->getId();
        self::assertNull($paymentRecord->getPayerAccount());

        $this->deliverPaymentIntentSucceeded((string) $paymentRecord->getStripePaymentIntentId());

        $this->activateTenant($trainer);
        $confirmed = $submissions->find($submission->getId());
        self::assertNotNull($confirmed);

        $crawler2 = $this->client->request('GET', $this->convertAccountUrl($confirmed));
        $accountForm = $crawler2->selectButton('Create Account')->form([
            'convert_submission_to_account[plainPassword][first]' => 'correct-horse-battery',
            'convert_submission_to_account[plainPassword][second]' => 'correct-horse-battery',
            'convert_submission_to_account[dateOfBirth]' => sprintf('%d-06-01', ((int) date('Y')) - 28),
            'convert_submission_to_account[acceptTerms]' => true,
        ]);
        $this->client->submit($accountForm);
        self::assertResponseRedirects('/dashboard');

        $account = $this->account('a5-payer@example.test');

        $this->activateTenant($trainer);
        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $reloadedPaymentRecord = $paymentRecords->find($paymentRecordId);
        self::assertNotNull($reloadedPaymentRecord);
        self::assertSame($account->getId(), $reloadedPaymentRecord->getPayerAccount()?->getId(), 'A5: the existing payment record is updated in place, never duplicated.');
        self::assertSame(PaymentRecord::TYPE_CAMP_REGISTRATION, $reloadedPaymentRecord->getType());
    }

    private function confirmedFreeSubmission(string $name, string $email): FormSubmission
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Conversion Camp '.uniqid()]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => $name,
            'form_submission[participant_email]' => $email,
        ]);
        $this->client->submit($submissionForm);

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, $email);
        self::assertNotNull($submission);

        return $submission;
    }

    private function convertAccountUrl(FormSubmission $submission): string
    {
        /** @var \App\Forms\Service\SubmissionTokenFactory $tokens */
        $tokens = self::getContainer()->get(\App\Forms\Service\SubmissionTokenFactory::class);

        return sprintf(
            '/forms/%s/convert-account?submission=%s',
            $submission->getForm()->getShareableSlug(),
            $tokens->tokenFor($submission),
        );
    }
}
