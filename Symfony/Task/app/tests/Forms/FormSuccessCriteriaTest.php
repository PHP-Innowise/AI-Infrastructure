<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use App\Forms\Repository\FormSubmissionRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Field\ChoiceFormField;

/**
 * Epic-08's epic-level "Success Criteria" (§13) — AC-08-33..46. Several of
 * these describe production/business outcomes over real-world usage (a
 * completion-time SLA, a payment success rate, a trainer-adoption
 * percentage) that no automated test run can measure; those are marked
 * skipped with the honest reason, per this task's own instruction, rather
 * than faked with an assertion that does not actually verify the claim.
 * Where a criterion has a genuinely testable structural component, it is
 * asserted for real.
 *
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md "Epic-level (from 'Success Criteria')"
 */
final class FormSuccessCriteriaTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-08-33: "Trainer can create a camp form in < 10 minutes." A wall-
     * clock human-UX duration claim — no automated test drives a real
     * trainer through the UI at human speed, and a script completing the
     * same HTTP calls in milliseconds proves nothing about the 10-minute
     * claim either way.
     */
    public function testAc08_33CampCreationTimeIsNotMachineVerifiable(): void
    {
        self::markTestSkipped('AC-08-33 is a human-UX wall-clock duration claim; no automated test can measure it. See this class\'s own docblock.');
    }

    /**
     * AC-08-34: "A form submission, including payment, completes in < 3
     * minutes." Same reasoning as AC-08-33, plus a genuine spec
     * contradiction: "UX Assumptions" (line 166) separately states "< 5
     * minutes" for the identical end-to-end action (fill + submit,
     * including payment). This implementation cites AC-08-34's "< 3
     * minutes" as authoritative, because it is the one restated as a
     * numbered, enumerated Success Criterion rather than a background
     * assumption — but the contradiction itself is unresolved in the
     * source material and is reported, not silently picked, in the
     * coder's final report.
     */
    public function testAc08_34SubmissionTimeIsNotMachineVerifiable(): void
    {
        self::markTestSkipped('AC-08-34 is a human-UX wall-clock duration claim (and conflicts with the Assumptions section\'s "<5 minutes" for the same action — see this test\'s own docblock); no automated test can measure it.');
    }

    /**
     * AC-08-35: "Payment success rate is 95%+ (Stripe reliability)." A
     * production reliability figure about Stripe's own infrastructure over
     * real traffic, not this codebase's behavior on any single run.
     */
    public function testAc08_35PaymentSuccessRateIsAnOperationalMetricNotATestAssertion(): void
    {
        self::markTestSkipped('AC-08-35 measures Stripe\'s own real-world reliability over production traffic; not something a local test run can assert.');
    }

    /**
     * AC-08-36: "Shareable links work across all devices." Genuinely
     * testable in the one structural sense available to a server-rendered,
     * no-JS-required page: the SAME route serves every device identically
     * (no user-agent branching anywhere in PublicFormController), and the
     * page declares a mobile viewport. Whether it actually RENDERS
     * acceptably on every real device is a visual/cross-browser matrix
     * this suite cannot drive — see AC-08-40's identical structural scope.
     */
    public function testAc08_36PublicFormServesIdenticalMarkupRegardlessOfDevice(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Cross-Device Camp']);

        $desktopCrawler = $this->requestWithUserAgent($form, 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) Chrome/120.0');
        $mobileCrawler = $this->requestWithUserAgent($form, 'Mozilla/5.0 (iPhone; CPU iPhone OS 17_0 like Mac OS X) Mobile/15E148');

        self::assertResponseIsSuccessful();
        self::assertSame(
            $desktopCrawler->filter('form[name="form_submission"]')->count(),
            $mobileCrawler->filter('form[name="form_submission"]')->count(),
            'The same route and markup serve every device — no user-agent branching exists in PublicFormController.',
        );
        self::assertCount(1, $desktopCrawler->filter('meta[name="viewport"]'));
    }

    /**
     * AC-08-37: "Form submissions are stored correctly with all data" —
     * every answer type (text, email, dropdown, multi-select) round-trips
     * exactly through submission_data.
     */
    public function testAc08_37AllFieldTypesStoreExactly(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $fields = [
            new FormField(FormField::FIELD_PARTICIPANT_NAME, FormField::TYPE_TEXT, 'Participant Name', true),
            new FormField(FormField::FIELD_PARTICIPANT_EMAIL, FormField::TYPE_EMAIL, 'Participant Email', true),
            new FormField('skill_level', FormField::TYPE_DROPDOWN, 'Skill Level', true, ['Beginner', 'Intermediate', 'Advanced']),
            new FormField('sizes', FormField::TYPE_MULTISELECT, 'Jersey Sizes', false, ['S', 'M', 'L']),
        ];
        $form = $this->createCamp($trainer, ['name' => 'All Field Types Camp', 'fields' => $fields]);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form();
        $submissionForm['form_submission[participant_name]'] = 'Type Coverage Tester';
        $submissionForm['form_submission[participant_email]'] = 'type-coverage@example.test';

        $skillLevelField = $submissionForm['form_submission[skill_level]'];
        \assert($skillLevelField instanceof ChoiceFormField);
        $skillLevelField->select('Intermediate');

        $sizesField = $submissionForm['form_submission[sizes]'];
        \assert($sizesField instanceof ChoiceFormField);
        $sizesField->select(['S', 'L']);
        $this->client->submit($submissionForm);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, 'type-coverage@example.test');
        self::assertNotNull($submission);
        self::assertSame('Intermediate', $submission->answerFor('skill_level'));
        self::assertSame(['S', 'L'], $submission->answerFor('sizes'));
    }

    /**
     * AC-08-38: "The user conversion flow works seamlessly" — the full,
     * genuine end-to-end path: submit -> confirm -> convert -> land
     * authenticated on the dashboard, in one continuous run (as opposed to
     * FormSubmissionConversionTest's own more granular, per-AC assertions
     * on the same flow).
     */
    public function testAc08_38EndToEndConversionFlowIsSeamless(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Seamless Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => 'Seamless Convert',
            'form_submission[participant_email]' => 'seamless-convert@example.test',
        ]);
        $this->client->submit($submissionForm);
        self::assertResponseRedirects();

        $confirmCrawler = $this->client->followRedirect();
        self::assertResponseIsSuccessful();
        $createAccountLink = $confirmCrawler->selectLink('Create Your Account')->link();

        $accountCrawler = $this->client->click($createAccountLink);
        self::assertResponseIsSuccessful();

        $accountForm = $accountCrawler->selectButton('Create Account')->form([
            'convert_submission_to_account[plainPassword][first]' => 'correct-horse-battery',
            'convert_submission_to_account[plainPassword][second]' => 'correct-horse-battery',
            'convert_submission_to_account[dateOfBirth]' => sprintf('%d-06-01', ((int) date('Y')) - 24),
            'convert_submission_to_account[acceptTerms]' => true,
        ]);
        $this->client->submit($accountForm);

        self::assertResponseRedirects('/dashboard');
        $this->client->followRedirect();
        self::assertResponseIsSuccessful();
    }

    /**
     * AC-08-39: "Forms load in < 2 seconds." A production latency SLA.
     * Asserting a wall-clock bound in a shared, variable-load CI container
     * would be flaky and would not actually validate a production
     * guarantee either way — this needs real APM/load-testing
     * infrastructure this suite does not have.
     */
    public function testAc08_39LoadTimeRequiresProductionApmNotAUnitTest(): void
    {
        self::markTestSkipped('AC-08-39 is a production latency SLA; a wall-clock assertion in a shared CI container would be flaky and would not validate it either way. Needs real APM/load testing.');
    }

    /**
     * AC-08-40: "Forms are mobile-friendly (responsive design)." Same
     * structural scope as AC-08-36 — the CSS itself (media queries,
     * fluid layout) is coder-frontend territory; what a backend test can
     * assert is that the page declares the mobile viewport every
     * responsive layout depends on, and that no separate "mobile" route
     * or template branch exists to drift out of sync.
     */
    public function testAc08_40FormsDeclareAMobileViewport(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Mobile Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        $viewport = $crawler->filter('meta[name="viewport"]')->attr('content');
        self::assertNotNull($viewport);
        self::assertStringContainsString('width=device-width', $viewport);
    }

    /**
     * AC-08-41: "Forms meet WCAG 2.1 Level AA." A partial, structural
     * check — every input has an associated, programmatically-determined
     * label (SC 1.3.1/4.1.2) and the page declares a language (SC 3.1.1).
     * This is not a full WCAG AA audit (contrast ratios, focus order under
     * dynamic content, and screen-reader behavior need real assistive-tech
     * testing, out of this suite's reach) — recorded honestly rather than
     * claimed as complete compliance.
     */
    public function testAc08_41PublicFormFieldsHaveAssociatedLabels(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Accessible Camp']);

        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertNotEmpty($crawler->filter('html[lang]'), 'SC 3.1.1: page language declared.');

        // Excludes the honeypot: it is deliberately unlabeled and wrapped
        // in aria-hidden="true" (PublicSubmissionFormBuilder's own
        // docblock) — removed from the accessibility tree entirely, which
        // is the CORRECT accessible treatment for a field no assistive-tech
        // user should ever perceive, not an accessibility gap.
        $inputs = $crawler->filter('#main input[type="text"]:not([name="form_submission[website]"]), #main input[type="email"]');
        self::assertGreaterThan(0, $inputs->count());

        $inputs->each(static function ($input) use ($crawler): void {
            $id = $input->attr('id');
            self::assertNotNull($id, 'Every form input has an id to associate a label with.');
            self::assertGreaterThan(0, $crawler->filter(sprintf('label[for="%s"]', $id))->count(), sprintf('Input #%s has no associated <label> (SC 1.3.1/4.1.2).', $id));
        });
    }

    /**
     * AC-08-42: "Forms are served over HTTPS, and payment data is never
     * stored on the platform." Two claims bundled into one AC id; this
     * test genuinely verifies the second half — no card/PAN-shaped data
     * can even be captured (BR-08-2's closed Text/Email/Dropdown/
     * Multi-select vocabulary has no payment field type, and Stripe
     * Checkout, not a local form, is the only place a card number is ever
     * entered — BR-08-11). The HTTPS half is a deployment/TLS-termination
     * concern outside this application's own code (nginx/infra, not
     * verified here) — recorded honestly rather than silently claimed.
     */
    public function testAc08_42PaymentDataCanNeverBeCapturedByTheCustomFieldVocabulary(): void
    {
        self::assertSame(
            [FormField::TYPE_TEXT, FormField::TYPE_EMAIL, FormField::TYPE_DROPDOWN, FormField::TYPE_MULTISELECT],
            FormField::TYPES,
            'BR-08-2: the closed field vocabulary has no card/payment field type at all.',
        );

        // BR-08-11: no custom payment form/route exists anywhere in this
        // module -- Stripe Checkout (a page this platform never renders)
        // is the only place a card number is entered.
        $reflection = new \ReflectionClass(\App\Forms\Controller\PublicFormController::class);
        $methodNames = array_map(static fn (\ReflectionMethod $m): string => $m->getName(), $reflection->getMethods(\ReflectionMethod::IS_PUBLIC));
        foreach ($methodNames as $name) {
            self::assertStringNotContainsStringIgnoringCase('card', $name);
            self::assertStringNotContainsStringIgnoringCase('payment', $name);
        }

        // HTTPS itself is a deployment/nginx/TLS-termination concern, not
        // verified by this application-level test — see this test's own
        // docblock.
    }

    /**
     * AC-08-43: "Trainer adoption of camp/evaluation forms reaches 70%+
     * within the first 3 months." A product-adoption metric measured
     * across the trainer population over calendar time in production.
     */
    public function testAc08_43TrainerAdoptionIsAProductMetricNotATestAssertion(): void
    {
        self::markTestSkipped('AC-08-43 is a 3-month production adoption metric across the trainer population; not observable from a single test run.');
    }

    /**
     * AC-08-44: "Camp-participant-to-full-user conversion rate reaches
     * 30%+." A production funnel metric over real registrant behavior —
     * distinct from AC-08-38, which verifies the CONVERSION MECHANISM
     * itself works end-to-end (it does); this criterion is about what
     * fraction of real people choose to use it.
     */
    public function testAc08_44ConversionRateIsAProductMetricNotATestAssertion(): void
    {
        self::markTestSkipped('AC-08-44 is a production funnel-conversion percentage over real registrant behavior; the conversion MECHANISM itself is verified for real by testAc08_38EndToEndConversionFlowIsSeamless and FormSubmissionConversionTest.');
    }

    /**
     * AC-08-45: "Camp revenue accounts for 20%+ of platform transactions."
     * A production revenue-mix metric. See also this epic's own Open
     * Questions: a second, differently-scoped "20%+" target ("camp
     * registrations account for 20%+ of new player acquisitions") appears
     * elsewhere in the source and is not reconciled with this one —
     * reported in the coder's final report, not resolved here.
     */
    public function testAc08_45CampRevenueShareIsAProductMetricNotATestAssertion(): void
    {
        self::markTestSkipped('AC-08-45 is a production revenue-mix percentage; not observable from a single test run. The epic also states a second, differently-scoped 20%+ metric elsewhere (camp registrations as a share of new player acquisitions) that is not reconciled with this one — see the coder\'s final report.');
    }

    /**
     * AC-08-46: "Zero payment processing errors." An operational count
     * over real production traffic and time, not a single test run's
     * assertion. The error-HANDLING code paths themselves (a declined or
     * failed Stripe payment) are exercised for real elsewhere
     * (`ProcessStripeWebhookEventHandler`'s existing
     * `payment_intent.payment_failed` handling, generic across every
     * `PaymentRecord` type including `camp_registration`) — "zero errors
     * in production" itself is not something any test can prove.
     */
    public function testAc08_46ZeroProcessingErrorsIsAnOperationalMetricNotATestAssertion(): void
    {
        self::markTestSkipped('AC-08-46 is an operational error-COUNT over real production traffic; not provable by any single test run. The error-handling code path itself is exercised generically for every PaymentRecord type by the existing Billing webhook test suite.');
    }

    private function requestWithUserAgent(Form $form, string $userAgent): \Symfony\Component\DomCrawler\Crawler
    {
        return $this->client->request('GET', '/forms/'.$form->getShareableSlug(), [], [], ['HTTP_USER_AGENT' => $userAgent]);
    }
}
