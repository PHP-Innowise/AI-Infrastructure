<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\Yaml\Yaml;

/**
 * Three epic-level criteria that are not ordinary product behavior and say
 * so themselves: AC-01-64's camp-conversion half sits on an integration
 * point this spec explicitly puts out of scope, AC-01-77 states numeric
 * targets no functional-test suite can honestly certify, and AC-01-78 is
 * marked "(Process gate, not product behavior)" in its own text. Per the
 * task brief: implement/verify everything that IS reachable, and record
 * the rest as a documented conflict rather than fabricate a passing
 * assertion for something this suite cannot actually prove.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-64, AC-01-77, AC-01-78
 */
final class EpicCompletionCriteriaTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-01-64: "alternatively a ShareLink can be sent by email for later
     * registration" — the half of this criterion that is actually Epic-01's
     * to build. It is not a separate mechanism: it is the same coach-invite
     * ShareLink flow already exercised end to end by
     * CoachInvitationTest::testTrainerInvitesACoachByEmail (unique code,
     * emailed, followed, registers, associates). This test only confirms
     * the general-purpose entry point a camp/evaluation submitter would be
     * pointed at also works for an arbitrary target email, i.e. nothing
     * about it is coach-specific in a way that would block reuse from
     * Epic-08.
     *
     * The other half — "the system prompts the submitter to create an
     * account [and] pre-fills the registration form with the camp
     * submission data" — requires a camp/evaluation form to submit in the
     * first place. That is Epic-08's own object model (`camp_registration`,
     * per open-questions register decision A3), which does not exist yet.
     * The spec says as much in its own text: "Epic-08's side of the
     * integration is out of scope for this spec." Nothing here pretends
     * otherwise.
     */
    public function testShareLinkByEmailFallbackWorksAsTheCampConversionAlternative(): void
    {
        $trainer = $this->trainer('peak-performance');
        $trainerOwner = $trainer->getOwnerAccount();
        $this->activateTenant($trainer);

        /** @var \App\Identity\Service\ShareLinkService $shareLinkService */
        $shareLinkService = self::getContainer()->get(\App\Identity\Service\ShareLinkService::class);
        $link = $shareLinkService->issueCoachInvite($trainer, $trainerOwner, 'camp-submitter@example.test');

        self::assertTrue($link->isUsable(new \DateTimeImmutable()), 'AC-01-64: a freshly issued email ShareLink is immediately usable.');
        self::assertSame('camp-submitter@example.test', $link->getTargetEmail());

        // Following it registers and auto-associates with the issuing
        // trainer — the "auto-assigns the new account to the trainer after
        // creation" half of AC-01-64, proven the same way US-01.08 proves it
        // for a coach invite.
        $this->client->request('GET', '/invite/'.$link->getCode());
        self::assertResponseIsSuccessful();
        self::assertSelectorExists('form[name="coach_registration"]');
    }

    /**
     * AC-01-77: numeric performance targets (dashboard <2s, 10,000-user list
     * <3s, profile edits <1s, 1,000 concurrent users). What follows is a
     * smoke-level regression guard against the two single-request targets,
     * using the spec's own numbers as the bound — useful for catching a
     * gross regression (e.g. an N+1 query), but NOT a substitute for real
     * load testing: a single request on a shared dev/CI container proves
     * nothing about production hardware, network, or concurrent load.
     *
     * The other two targets are not attempted, rather than faked:
     *  - a 10,000-user list needs a seeded 10,000-row dataset this suite's
     *    fixtures do not build (and should not, as a side effect of a
     *    correctness suite);
     *  - "1,000 concurrent users" requires a load-testing tool that drives
     *    real concurrent connections (k6, JMeter, Gatling) — PHPUnit runs
     *    one request at a time in one process and cannot produce that
     *    condition at all, honestly or otherwise.
     */
    public function testDashboardAndProfileEditCompleteWithinTheirStatedBudgetsAsASmokeCheck(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $start = microtime(true);
        $this->client->request('GET', '/dashboard');
        $dashboardSeconds = microtime(true) - $start;

        self::assertResponseIsSuccessful();
        self::assertLessThan(2.0, $dashboardSeconds, 'AC-01-77 smoke check: dashboard target is <2s.');

        $crawler = $this->client->request('GET', '/account/profile');
        $form = $crawler->selectButton('Save changes')->form([
            'edit_profile[firstName]' => 'Timed',
            'edit_profile[lastName]' => 'Save',
        ]);

        $start = microtime(true);
        $this->client->submit($form);
        $saveSeconds = microtime(true) - $start;

        self::assertResponseRedirects();
        self::assertLessThan(1.0, $saveSeconds, 'AC-01-77 smoke check: profile edits target is <1s.');
    }

    /**
     * AC-01-77's two genuinely un-testable-here targets, marked skipped
     * (not silently omitted, not faked) so the reason is visible in every
     * test run rather than only in this docblock.
     */
    public function testUserListAtTenThousandRowsAndConcurrencyTargetsAreOutOfThisSuitesReach(): void
    {
        self::markTestSkipped(
            'AC-01-77: "10,000-user list loads in <3s with pagination" and '.
            '"platform supports 1,000 concurrent users" require a seeded '.
            '10k-row dataset and a real concurrent-load tool respectively — '.
            'neither is something a single-process PHPUnit functional test '.
            'can honestly certify. Needs a dedicated load-testing pass '.
            '(e.g. k6/JMeter against a staging deploy), tracked outside this suite.',
        );
    }

    /**
     * AC-01-78 ("Process gate, not product behavior" — the spec's own
     * words): "Epic-01 is considered complete only once the demo is
     * approved, all P0 open questions are resolved, and the security review
     * has passed if applicable."
     *
     * "All P0 open questions are resolved" IS machine-checkable, against the
     * one artifact of record for that fact: the consolidated open-questions
     * register's Section A is the project's own definition of "P0/blocking"
     * (its heading: "the Phase 1 blocking checkpoint: Phase 2 (design)
     * should not start until Section A is answered"), and it is dated and
     * marked answered. This test reads that file rather than asserting a
     * fact about it from memory, so it breaks the moment that stops being
     * true rather than silently going stale.
     */
    public function testAllP0OpenQuestionsAreRecordedAsResolved(): void
    {
        $projectDir = self::getContainer()->getParameter('kernel.project_dir');

        // specs/ lives at the monorepo root, one level above this app.
        $registerPath = \dirname($projectDir).'/specs/requirements-analyst-open-questions.md';
        self::assertFileExists($registerPath, 'The consolidated open-questions register should exist at the expected path.');

        $contents = file_get_contents($registerPath);
        self::assertIsString($contents);

        self::assertMatchesRegularExpression(
            '/## Section A [—-]+ ANSWERED/',
            $contents,
            'AC-01-78: Section A (the P0/blocking checkpoint) must be recorded as answered, not left open.',
        );
        self::assertStringContainsStringIgnoringCase(
            'Do not relitigate these',
            $contents,
            'Section A should be closed, not merely drafted.',
        );
    }

    /**
     * AC-01-78's other two clauses are human process steps by definition —
     * a demo approval and a security review sign-off are not things that
     * happen inside an application's own behavior, so no assertion against
     * this codebase could ever honestly stand in for them.
     */
    public function testDemoApprovalAndSecurityReviewAreHumanProcessGatesNotAutomatable(): void
    {
        self::markTestSkipped(
            'AC-01-78: "the demo is approved" and "the security review has '.
            'passed if applicable" are sign-offs by people outside this '.
            'codebase, not application behavior — tracked as project '.
            'process, not asserted here.',
        );
    }
}
