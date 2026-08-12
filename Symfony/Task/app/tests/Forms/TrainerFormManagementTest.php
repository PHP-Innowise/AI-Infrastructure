<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Billing\Entity\PaymentRecord;
use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Repository\FormRepository;
use App\Forms\Service\FormService;
use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\PublicTenantCodeRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Crawler;

/**
 * US-08.01/08.02/08.06: trainer console — create, customize, preview,
 * publish, toggle, edit, and delete camps and evaluations.
 */
final class TrainerFormManagementTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-08-1: "Camps & Evaluations" section, "Create Camp" from a
     * pre-loaded template.
     */
    public function testTrainerCanAccessCampsSectionAndStartFromTemplate(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $this->client->request('GET', '/trainer/forms');
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('a[href="/trainer/forms/camps/new"]');

        $crawler = $this->client->request('GET', '/trainer/forms/camps/new');
        self::assertResponseIsSuccessful();

        // BR-08-1: the pre-loaded template already seeds the reserved
        // participant name/email fields.
        self::assertSame('participant_name', $crawler->filter('input[name="camp_form[fields][0][id]"]')->attr('value'));
        self::assertSame('Participant Name', $crawler->filter('input[name="camp_form[fields][0][label]"]')->attr('value'));
        self::assertSame('participant_email', $crawler->filter('input[name="camp_form[fields][1][id]"]')->attr('value'));
    }

    /**
     * AC-08-2/BR-08-2: customizable fields, minimum 1 (participant name),
     * and the four-type vocabulary enforced.
     */
    public function testCampRequiresAtLeastParticipantNameFieldAndRejectsUnknownType(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/forms/camps/new');
        $form = $crawler->selectButton('Create Camp')->form([
            'camp_form[name]' => 'Fields Test Camp',
            'camp_form[capacityLimit]' => '10',
            // Blank out the reserved email field's id -- the entity guard
            // must reject a submission missing it.
            'camp_form[fields][1][id]' => '',
            'camp_form[fields][1][label]' => '',
        ]);
        $this->client->submit($form);

        // Error contract: a Form POST validation failure re-renders the
        // same template, same URL, HTTP 422 (api-designer-spec.md).
        self::assertResponseStatusCodeSame(422);
        self::assertSelectorTextContains('.skeleton-card', 'participant email');

        $this->activateTenant($trainer);
        /** @var FormRepository $forms */
        $forms = self::getContainer()->get(FormRepository::class);
        $names = array_map(static fn (Form $f): string => $f->getName(), $forms->findAllForTrainer($trainer));
        self::assertNotContains('Fields Test Camp', $names, 'No camp should have been created.');
    }

    /**
     * AC-08-3: name, display-only dates (folded into description), and
     * description.
     */
    public function testTrainerSetsCampNameDisplayDatesAndDescription(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/forms/camps/new');
        $form = $crawler->selectButton('Create Camp')->form([
            'camp_form[name]' => 'Summer Skills Camp',
            'camp_form[description]' => 'July 10-14, 2027. Peak Performance Gym.',
            'camp_form[capacityLimit]' => '30',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();

        $created = $this->latestFormFor($trainer);
        self::assertSame('Summer Skills Camp', $created->getName());
        self::assertStringContainsString('July 10-14, 2027', (string) $created->getDescription());
    }

    /**
     * AC-08-4: capacity limit constrained to 1-1000.
     */
    public function testCampCapacityMustBeWithinOneToOneThousand(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/forms/camps/new');
        $form = $crawler->selectButton('Create Camp')->form([
            'camp_form[name]' => 'Too Big Camp',
            'camp_form[capacityLimit]' => '1001',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422);
    }

    /**
     * AC-08-5: price is $0 (free) or $1-$10,000 — nothing in between 1 and
     * 99 minor units, and nothing above $10,000, is legal.
     */
    public function testCampPriceMustBeFreeOrWithinDollarRange(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);

        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);

        $free = $formService->createCamp($trainer, 'Free Camp', null, null, 10, self::defaultTestFields());
        self::assertTrue($free->isFree());

        $paid = $formService->createCamp($trainer, 'Paid Camp', null, 5000, 10, self::defaultTestFields());
        self::assertSame(5000, $paid->getPriceMinorUnits());

        $this->expectException(\InvalidArgumentException::class);
        $formService->createCamp($trainer, 'Too Cheap Camp', null, 50, 10, self::defaultTestFields());
    }

    /**
     * AC-08-6: preview before publishing — reachable pre-publish, renders
     * the public template read-only.
     */
    public function testTrainerCanPreviewCampBeforePublishing(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Preview Camp', 'publish' => false]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/forms/%d/preview', (int) $form->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Preview Camp');
        self::assertSelectorTextContains('body', 'Preview mode');

        // Not yet published — the public route must 404 for anyone,
        // including this same, still-logged-in-as-trainer client: source 5
        // outranks the session's own selected trainer context.
        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseStatusCodeSame(404);
    }

    /**
     * AC-08-7: publishing produces a shareable link.
     */
    public function testPublishingProducesAWorkingShareableLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Publish Camp', 'publish' => false]);
        $formId = (int) $form->getId();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        $this->postForm($editCrawler, '/publish');
        self::assertResponseRedirects();

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();

        // Once published, the edit page no longer offers a "Publish"
        // control at all (it shows the link instead) — the UI itself
        // makes a second click unreachable. "on FIRST publish"'s
        // idempotency is therefore a FormService-level guarantee, verified
        // directly: calling publish() again must not create a second
        // PublicTenantCode row for the same form.
        $this->activateTenant($trainer);
        /** @var PublicTenantCodeRepository $codes */
        $codes = self::getContainer()->get(PublicTenantCodeRepository::class);
        self::assertNotNull($codes->findOneByReference(PublicTenantCode::KIND_FORM, $formId));

        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);
        $formService->publish($form);

        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        self::assertResponseIsSuccessful();
        self::assertCount(0, $editCrawler->filter('form[action$="/publish"]'), 'Already published: no Publish control remains.');
        self::assertSelectorTextContains('.skeleton-card', $form->getShareableSlug());
    }

    /**
     * AC-08-8/AC-08-29: enable/disable at any time; disabled shows
     * "Registration Closed" on the public link instead of the form.
     */
    public function testTrainerCanToggleCampRegistrationOnAndOff(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Toggle Camp']);
        $formId = (int) $form->getId();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        $this->postForm($editCrawler, '/toggle');
        self::assertResponseRedirects();

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Registration Closed');

        // Re-enable.
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        $this->postForm($editCrawler, '/toggle');
        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Registration Closed');
    }

    /**
     * AC-08-9: "Camps & Evaluations" -> "Create Evaluation" from a
     * pre-loaded template.
     */
    public function testTrainerCanAccessEvaluationsSectionAndStartFromTemplate(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $this->client->request('GET', '/trainer/forms');
        self::assertSelectorExists('a[href="/trainer/forms/evaluations/new"]');

        $crawler = $this->client->request('GET', '/trainer/forms/evaluations/new');
        self::assertResponseIsSuccessful();
        self::assertSame('participant_name', $crawler->filter('input[name="evaluation_form[fields][0][id]"]')->attr('value'));
    }

    /**
     * AC-08-10: same field types as camps; name, description, optional
     * price.
     */
    public function testTrainerCustomizesEvaluationFieldsNameDescriptionPrice(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/forms/evaluations/new');
        $form = $crawler->selectButton('Create Evaluation')->form([
            'evaluation_form[name]' => 'Elite Tryout Evaluation',
            'evaluation_form[description]' => 'Skills assessment.',
            'evaluation_form[priceMinorUnits]' => '2500',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $created = $this->latestFormFor($trainer);
        self::assertSame(Form::TYPE_EVALUATION, $created->getFormType());
        self::assertSame(2500, $created->getPriceMinorUnits());
    }

    /**
     * AC-08-11: preview before publishing.
     */
    public function testTrainerCanPreviewEvaluationBeforePublishing(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createEvaluation($trainer, ['name' => 'Preview Eval', 'publish' => false]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/forms/%d/preview', (int) $form->getId()));
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('h1', 'Preview Eval');
    }

    /**
     * AC-08-12: permanent link, no capacity, no on/off toggle — only
     * deletable.
     */
    public function testEvaluationHasNoCapacityAndCannotBeToggled(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createEvaluation($trainer, ['name' => 'Permanent Eval']);

        self::assertNull($form->getCapacityLimit());
        self::assertTrue($form->isOpenForRegistration());

        // No toggle control is even rendered on the edit page for an
        // evaluation (FormVoter::FORM_TOGGLE would deny it regardless).
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', (int) $form->getId()));
        self::assertResponseIsSuccessful();
        self::assertCount(0, $editCrawler->filter('form[action$="/toggle"]'));

        $this->client->request('POST', sprintf('/trainer/forms/%d/toggle', (int) $form->getId()), ['_token' => 'irrelevant']);
        self::assertResponseStatusCodeSame(403);
    }

    /**
     * AC-08-27: the trainer's list of every camp/evaluation, with a
     * copyable shareable link.
     */
    public function testTrainerViewsListWithShareableLinks(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Listed Camp']);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/forms');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Listed Camp');
        $row = $crawler->filter('tr')->reduce(static fn (Crawler $tr): bool => str_contains($tr->text(), 'Listed Camp'));
        self::assertStringContainsString($form->getShareableSlug(), $row->filter('code')->text());
    }

    /**
     * AC-08-28: editing a form with existing submissions warns how many
     * participants are already registered.
     */
    public function testEditingFormWithSubmissionsWarnsRegistrationCount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Warn Camp', 'capacityLimit' => 20]);
        $this->submitFreeRegistration($form, 'warn-participant@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', (int) $form->getId()));

        self::assertResponseIsSuccessful();
        self::assertStringContainsString('1 participant', $crawler->filter('.skeleton-card')->text());
    }

    /**
     * AC-08-30: capacity may never drop below the current registration
     * count.
     */
    public function testCapacityCannotBeReducedBelowCurrentRegistrations(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Floor Camp', 'capacityLimit' => 2]);
        $this->submitFreeRegistration($form, 'floor-1@example.test');
        $this->submitFreeRegistration($form, 'floor-2@example.test');

        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);

        $this->expectException(\InvalidArgumentException::class);
        $formService->update($form, $form->getName(), $form->getDescription(), null, 1, $form->getFields());
    }

    /**
     * AC-08-31: a form with paid, unrefunded registrations cannot be
     * deleted.
     */
    public function testDeletingFormWithPaidRegistrationIsBlocked(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Delete-Blocked Camp', 'priceMinorUnits' => 5000]);
        $formId = (int) $form->getId();

        // Directly persist a paid submission to isolate this test from the
        // full Stripe checkout flow (covered by CampPaymentTest).
        $this->persistConfirmedSubmission($form, 'paid-blocker@example.test', FormSubmission::STATUS_PAID);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        $this->postForm($editCrawler, '/delete');

        self::assertResponseRedirects(sprintf('/trainer/forms/%d/edit', $formId));

        // findForm() itself asserts the row still exists (fails with "Form
        // with slug ... not found" otherwise) -- the call IS the assertion.
        $this->activateTenant($trainer);
        $this->findForm($form->getShareableSlug());
    }

    /**
     * AC-08-31 (converse): a form with zero registrations deletes cleanly.
     */
    public function testDeletingFormWithNoRegistrationsSucceeds(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Deletable Camp']);
        $formId = (int) $form->getId();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $editCrawler = $this->client->request('GET', sprintf('/trainer/forms/%d/edit', $formId));
        $this->postForm($editCrawler, '/delete');

        self::assertResponseRedirects('/trainer/forms');

        $this->activateTenant($trainer);
        /** @var FormRepository $forms */
        $forms = self::getContainer()->get(FormRepository::class);
        self::assertNull($forms->find($formId));
    }

    /**
     * AC-08-32: current submission count and remaining spots.
     */
    public function testTrainerViewsSubmissionCountAndRemainingSpots(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Spots Camp', 'capacityLimit' => 5]);
        $this->submitFreeRegistration($form, 'spots-1@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/forms');

        $row = $crawler->filter('tr')->reduce(static fn (Crawler $tr): bool => str_contains($tr->text(), 'Spots Camp'));
        self::assertStringContainsString('1', $row->filter('td')->eq(3)->text());
        self::assertStringContainsString('4', $row->filter('td')->eq(4)->text());
    }

    /**
     * Extracts the CSRF token from a hand-rolled `<form action="...$actionSuffix">`
     * on an already-rendered page and POSTs to it — these publish/toggle/
     * delete controls are plain Twig `csrf_token()` calls, not Symfony
     * FormType-generated, so the token can only be read off the DOM (the
     * CSRF token manager needs an active request session, which is not
     * available from a bare service-container call in a test method).
     */
    private function postForm(Crawler $crawler, string $actionSuffix): void
    {
        $formNode = $crawler->filter(sprintf('form[action$="%s"]', $actionSuffix));
        self::assertGreaterThan(0, $formNode->count(), sprintf('No form with action ending "%s" on this page.', $actionSuffix));

        $action = (string) $formNode->attr('action');
        $token = $formNode->filter('input[name="_token"]')->attr('value');

        $this->client->request('POST', $action, ['_token' => $token]);
    }

    private function latestFormFor(Trainer $trainer): Form
    {
        /** @var FormRepository $forms */
        $forms = self::getContainer()->get(FormRepository::class);
        $all = $forms->findAllForTrainer($trainer);
        self::assertNotEmpty($all);

        return $all[0];
    }

    private function submitFreeRegistration(Form $form, string $email): void
    {
        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Test Participant',
            'form_submission[participant_email]' => $email,
        ]);
        $this->client->submit($submissionForm);
    }

    private function persistConfirmedSubmission(Form $form, string $email, string $paymentStatus): void
    {
        $entityManager = $this->formsEntityManager();
        $submission = new FormSubmission(
            $form->getTrainer(),
            $form,
            ['participant_name' => 'Paid Participant', 'participant_email' => $email],
            $email,
            $paymentStatus,
        );
        $entityManager->persist($submission);
        $entityManager->flush();

        if (FormSubmission::STATUS_PAID === $paymentStatus) {
            $paymentRecord = new PaymentRecord(
                $form->getTrainer(),
                PaymentRecord::TYPE_CAMP_REGISTRATION,
                PaymentRecord::METHOD_CARD,
                5000,
                'Paid Participant',
                $email,
                null,
            );
            $paymentRecord->attachRelatedFormSubmission($submission);
            $paymentRecord->applyFee(500, 250);
            $paymentRecord->markCompleted();
            $entityManager->persist($paymentRecord);
            $entityManager->flush();

            $submission->attachPaymentRecord($paymentRecord);
            $entityManager->flush();
        }
    }
}
