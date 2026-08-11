<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Repository\FormSubmissionRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Mime\Email;

/**
 * US-08.03: the public, unauthenticated camp/evaluation registration flow —
 * AC-08-13..16, BR-08-6/7/9/10.
 */
final class PublicFormSubmissionTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-08-13: name, dates, description, price, spots remaining, with no
     * user authenticated at all.
     */
    public function testAnonymousVisitorSeesFullCampDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, [
            'name' => 'Details Camp',
            'description' => 'August 1-5. All skill levels.',
            'capacityLimit' => 25,
            'priceMinorUnits' => 1500,
        ]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Details Camp');
        self::assertSelectorTextContains('body', 'August 1-5');
        self::assertSelectorTextContains('body', '$15.00');
        self::assertSelectorTextContains('body', '25 spot(s) remaining');
        // Genuinely anonymous: no session-authenticated user context.
        self::assertNull($this->client->getRequest()->getSession()->get('_security_main') ?? null);
        unset($crawler);
    }

    /**
     * AC-08-13, regression. The defect this pins was real, and it was not a
     * Forms bug: TenantResolver's source-4 fallthrough resolves a logged-in
     * trainer to their *own* tenant, so on a code-resolved public route the
     * tenant could resolve from the session even when the code itself
     * resolved nothing. An unpublished camp's public link was therefore live
     * — but only for the one person who must not be misled about it, its
     * owner, who would see a working page and reasonably conclude it had
     * shipped.
     *
     * The fix re-checks publication explicitly on every public action rather
     * than trusting resolution, which is the council's "resolution is not
     * authorization" rule applied here. An anonymous visitor already 404s
     * because nothing resolves at all; this asserts the harder half.
     */
    public function testAnUnpublishedFormIs404EvenForItsOwnLoggedInTrainer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Draft Camp', 'publish' => false]);
        $code = $form->getShareableSlug();

        // Anonymous: nothing resolves, so this half was never in doubt.
        $this->client->request('GET', '/forms/'.$code);
        self::assertResponseStatusCodeSame(404, 'An unpublished form must not be publicly reachable.');

        // The owning trainer, signed in — the case that used to leak a live
        // page, because their session resolved the tenant the code did not.
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/forms/'.$code);

        self::assertResponseStatusCodeSame(
            404,
            'A signed-in trainer must not see their own unpublished form as a live public page: '
            .'resolving a tenant from the session is not permission to publish.',
        );
    }

    /**
     * AC-08-13: an unknown code 404s.
     */
    public function testUnknownCode404s(): void
    {
        $this->client->request('GET', '/forms/does-not-exist');
        self::assertResponseStatusCodeSame(404);
    }

    /**
     * AC-08-14/BR-08-9: fields captured, email format validated, no
     * partial submission accepted.
     */
    public function testSubmissionRequiresAllFieldsAndValidEmailFormat(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Validation Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Val Test',
            'form_submission[participant_email]' => 'not-an-email',
        ]);
        $this->client->submit($submissionForm);

        self::assertResponseStatusCodeSame(422);

        $crawler2 = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm2 = $crawler2->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => '',
            'form_submission[participant_email]' => 'valid@example.test',
        ]);
        $this->client->submit($submissionForm2);
        self::assertResponseStatusCodeSame(422);
    }

    /**
     * AC-08-15 (free path)/AC-08-16: immediate confirmation, confirmation
     * email sent, data stored correctly (AC-08-37).
     */
    public function testFreeCampConfirmsImmediatelyAndStoresSubmissionData(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Free Confirm Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Riley Registrant',
            'form_submission[participant_email]' => 'riley.registrant@example.test',
            'form_submission[emergency_contact]' => 'Sam, 555-0199',
        ]);
        $this->client->submit($submissionForm);

        self::assertResponseRedirects();

        // Checked before followRedirect(): the mailer's collector reflects
        // only the request that just completed (matches
        // ShareLinkRegistrationTest's own precedent).
        self::assertQueuedEmailCount(1);
        $message = self::getMailerMessages()[0];
        self::assertInstanceOf(Email::class, $message);
        self::assertSame(['riley.registrant@example.test'], array_map(static fn ($a) => $a->getAddress(), $message->getTo()));

        $this->client->followRedirect();
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', "You're registered");
        self::assertSelectorTextContains('body', 'Create Your Account');

        // AC-08-37: exact data, all fields.
        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'riley.registrant@example.test');
        self::assertNotNull($submission);
        self::assertSame('Riley Registrant', $submission->answerFor('participant_name'));
        self::assertSame('riley.registrant@example.test', $submission->answerFor('participant_email'));
        self::assertSame('Sam, 555-0199', $submission->answerFor('emergency_contact'));
        self::assertSame(FormSubmission::STATUS_FREE, $submission->getPaymentStatus());
        self::assertTrue($submission->isConfirmed());
    }

    /**
     * BR-08-10: the same email cannot submit the same form twice.
     */
    public function testDuplicateEmailIsRejected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Dup Camp']);

        $this->submit($form, 'dup-participant@example.test');

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Second Attempt',
            'form_submission[participant_email]' => 'dup-participant@example.test',
        ]);
        $this->client->submit($submissionForm);

        self::assertResponseStatusCodeSame(422);
        self::assertSelectorTextContains('.skeleton-card', 'already submitted');
    }

    /**
     * AC-08-15/BR-08-7: "Camp Full" and no submission accepted once
     * capacity is reached.
     */
    public function testCampFullBlocksFurtherSubmissions(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Tiny Camp', 'capacityLimit' => 1]);

        $this->submit($form, 'first-in@example.test');

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'full');
        self::assertCount(0, $crawler->filter('button:contains("Submit Registration")'));

        // A direct POST attempt is also blocked, not just the GET display
        // — rendered as the same friendly "Camp Full" page (never a bare
        // deny, which for an anonymous actor Symfony would otherwise turn
        // into a confusing redirect to /login).
        $this->client->request('POST', '/forms/'.$form->getShareableSlug(), [
            'form_submission' => ['participant_name' => 'Late Comer', 'participant_email' => 'late@example.test'],
        ]);
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'full');

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        self::assertNull($submissions->findOneByFormAndEmail($form, 'late@example.test'));
    }

    /**
     * "Risks & Mitigations — Form Spam Submissions": the honeypot field is
     * silently accepted-but-discarded, never a validation error, and
     * nothing is persisted.
     */
    public function testHoneypotFieldSilentlyDropsTheSubmission(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Honeypot Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertCount(1, $crawler->filter('input[name="form_submission[website]"]'), 'Honeypot field must be present.');

        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Bot Attempt',
            'form_submission[participant_email]' => 'bot@example.test',
            'form_submission[website]' => 'http://spam.example',
        ]);
        $this->client->submit($submissionForm);

        // Never a validation error -- looks like an ordinary redirect.
        self::assertResponseRedirects('/forms/'.$form->getShareableSlug());

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        self::assertNull($submissions->findOneByFormAndEmail($form, 'bot@example.test'), 'A honeypot-tripped submission must never be persisted.');
    }

    /**
     * AC-08-13/BR-08-5: an evaluation's link stays open with no capacity
     * concept and no "full" state.
     */
    public function testEvaluationHasNoCapacityConcept(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createEvaluation($trainer, ['name' => 'Open Evaluation']);

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'spot(s) remaining');
    }

    private function submit(Form $form, string $email): void
    {
        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Filler Participant',
            'form_submission[participant_email]' => $email,
        ]);
        $this->client->submit($submissionForm);
    }
}
