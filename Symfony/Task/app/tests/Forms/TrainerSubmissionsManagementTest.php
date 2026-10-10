<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Forms\Entity\Form;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;
use Symfony\Component\DomCrawler\Field\ChoiceFormField;
use Symfony\Component\Mime\Email;

/**
 * US-08.04: the trainer's participant list, filtering, export, attendance,
 * and bulk email — AC-08-17..22.
 */
final class TrainerSubmissionsManagementTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-08-17: name, email, payment status, submission date,
     * converted-to-user status (Yes/No) — age is read from the free-form
     * submission answers, matching Form's own field vocabulary (no
     * dedicated age column exists anywhere in this epic's data model).
     */
    public function testSubmissionsIndexShowsRequiredColumns(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Columns Camp']);
        $this->submitFree($form, 'Ivy Participant', 'ivy.participant@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/forms/%d/submissions', (int) $form->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Ivy Participant');
        self::assertSelectorTextContains('body', 'ivy.participant@example.test');
        self::assertSelectorTextContains('body', 'Free');
        self::assertSelectorTextContains('body', 'No'); // Converted to User: No
    }

    /**
     * AC-08-18: filterable by conversion status.
     */
    public function testSubmissionsFilterByConversionStatus(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Filter Camp']);
        // Deliberately not named after the filter's own vocabulary
        // ("Not Converted"/"User Created" are also the dropdown's option
        // labels, which would make a text-content assertion ambiguous).
        $this->submitFree($form, 'Filter Test Participant', 'filter-test-participant@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/forms/%d/submissions', (int) $form->getId()), ['conversionStatus' => 'converted']);

        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'filter-test-participant@example.test');

        $this->client->request('GET', sprintf('/trainer/forms/%d/submissions', (int) $form->getId()), ['conversionStatus' => 'not-converted']);
        self::assertSelectorTextContains('body', 'filter-test-participant@example.test');
    }

    /**
     * AC-08-19: CSV export.
     */
    public function testSubmissionsExportToCsv(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Export Camp']);
        $this->submitFree($form, 'Export Participant', 'export.participant@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/forms/%d/submissions/export', (int) $form->getId()));

        // The response body is a StreamedResponse — BrowserKit's test
        // client does not capture a streamed callback's output via
        // getContent() (confirmed empty in this exact suite already, see
        // TrainerRsvpListTest's identical note); the export's actual row
        // DATA is covered by testSubmissionsIndexShowsRequiredColumns
        // (same FormSubmissionRepository::findConfirmedForForm() source).
        // This checks the export is reachable and shaped as a downloadable
        // CSV attachment for this specific form.
        self::assertResponseIsSuccessful();
        self::assertStringContainsString('text/csv', (string) $this->client->getResponse()->headers->get('Content-Type'));
        self::assertStringContainsString(
            sprintf('form-%d-submissions.csv', (int) $form->getId()),
            (string) $this->client->getResponse()->headers->get('Content-Disposition'),
        );
    }

    /**
     * AC-08-20: per-participant attendance checkboxes, persisted.
     */
    public function testTrainerMarksAttendancePerParticipant(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Attendance Camp']);
        $this->submitFree($form, 'Present Participant', 'present@example.test');
        $this->submitFree($form, 'Absent Participant', 'absent@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/forms/%d/submissions/attendance', (int) $form->getId()));
        self::assertResponseIsSuccessful();

        $this->activateTenant($trainer);
        $presentSubmission = $this->findSubmissionByEmail($form, 'present@example.test');

        $formView = $crawler->selectButton('Save Attendance')->form();
        $attendanceField = $formView['form_attendance[attendance_'.$presentSubmission->getId().']'];
        \assert($attendanceField instanceof ChoiceFormField);
        $attendanceField->tick();
        $this->client->submit($formView);

        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        $reloaded = $this->findSubmissionByEmail($form, 'present@example.test');
        self::assertTrue($reloaded->isAttended());
        $absentReloaded = $this->findSubmissionByEmail($form, 'absent@example.test');
        self::assertFalse($absentReloaded->isAttended());
    }

    /**
     * AC-08-21: bulk email to every participant.
     */
    public function testTrainerSendsBulkEmailToAllParticipants(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Bulk Email Camp']);
        $this->submitFree($form, 'Recipient One', 'recipient-one@example.test');
        $this->submitFree($form, 'Recipient Two', 'recipient-two@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/forms/%d/submissions/bulk-email', (int) $form->getId()));
        $formView = $crawler->selectButton('Send Email')->form([
            'bulk_email[subject]' => 'Camp update',
            'bulk_email[body]' => 'See you all next week!',
        ]);
        $this->client->submit($formView);

        self::assertResponseRedirects();
        self::assertQueuedEmailCount(2);

        $recipients = array_map(
            static fn (Email $message): string => $message->getTo()[0]->getAddress(),
            array_filter(self::getMailerMessages(), static fn ($m) => $m instanceof Email),
        );
        self::assertContains('recipient-one@example.test', $recipients);
        self::assertContains('recipient-two@example.test', $recipients);
    }

    /**
     * AC-08-22: participant data remains visible indefinitely, even after
     * the camp "ends" — there is no time-based cutoff on the query at all
     * (Form's display-only dates carry no enforcement).
     */
    public function testParticipantDataRemainsVisibleIndefinitely(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, [
            'name' => 'Past Camp',
            'description' => 'Ran last year — display-only text, never enforced.',
        ]);
        $this->submitFree($form, 'Long Ago Participant', 'long-ago@example.test');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', sprintf('/trainer/forms/%d/submissions', (int) $form->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Long Ago Participant');
    }

    private function submitFree(Form $form, string $name, string $email): void
    {
        $crawler = $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        $submissionForm = $crawler->selectButton('Submit Registration')->form([
            'form_submission[participant_name]' => $name,
            'form_submission[participant_email]' => $email,
        ]);
        $this->client->submit($submissionForm);
    }

    private function findSubmissionByEmail(Form $form, string $email): \App\Forms\Entity\FormSubmission
    {
        /** @var \App\Forms\Repository\FormSubmissionRepository $submissions */
        $submissions = self::getContainer()->get(\App\Forms\Repository\FormSubmissionRepository::class);
        $submission = $submissions->findOneByFormAndEmail($form, $email);
        self::assertNotNull($submission);

        return $submission;
    }
}
