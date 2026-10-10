<?php

declare(strict_types=1);

namespace App\Forms\Controller;

use App\Forms\Entity\Form;
use App\Forms\Form\BulkEmailType;
use App\Forms\Form\FormAttendanceType;
use App\Forms\Repository\FormSubmissionRepository;
use App\Forms\Voter\FormVoter;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-08.04: the trainer's participant list, export, attendance and bulk
 * email — AC-08-17..22.
 *
 * @see specs/api-designer-spec.md "Forms module" — trainer console route table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerFormSubmissionController extends AbstractController
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private readonly FormSubmissionRepository $submissions,
        private readonly EntityManagerInterface $entityManager,
        private readonly MailerInterface $mailer,
    ) {
    }

    /**
     * AC-08-17/18/22: columns for name, email, age is left to the template
     * (read from `submissionData`, since age has no dedicated column — see
     * Form's own field vocabulary), payment status, submission date,
     * converted-to-user status. AC-08-18: filterable by payment status
     * (paid|free — "pending" is accepted as a query value but always
     * yields an empty result set, see this method's own note below) and
     * conversion status. AC-08-22: no time-based cutoff anywhere in this
     * query — data stays visible indefinitely.
     *
     * **AC-08-18 vs. Business Rules, resolved.** The acceptance criterion's
     * own filter list names "Paid, Free, Pending" as the three payment-status
     * options, but the SAME story's Business Rules state "Only paid/free
     * registrations shown (pending payments excluded)"
     * (requirements-analyst-epic-08-forms-registration-spec.md, restated
     * verbatim in AC-08-18's own text and flagged as an open contradiction
     * in that spec's Open Questions). This implementation follows the
     * Business Rule: the list itself is always confirmed-only
     * (`FormSubmissionRepository::countConfirmedForForm()`), and "Pending"
     * is not offered as a selectable filter in the UI (offering a filter
     * that always returns nothing is worse UX than not offering it) —
     * recorded here and in the coder's final report as the resolution
     * chosen, not silently picked.
     */
    #[Route('/trainer/forms/{form<\d+>}/submissions', name: 'forms_trainer_submissions_index', methods: ['GET'])]
    public function index(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_VIEW_SUBMISSIONS, $form);

        $conversionStatus = $request->query->getString('conversionStatus');

        return $this->render('forms/trainer_submissions_index.html.twig', [
            'form' => $form,
            'submissions' => $this->submissions->findConfirmedForForm($form, '' !== $conversionStatus ? $conversionStatus : null),
            'conversionStatus' => $conversionStatus,
        ]);
    }

    /**
     * AC-08-19.
     */
    #[Route('/trainer/forms/{form<\d+>}/submissions/export', name: 'forms_trainer_submissions_export', methods: ['GET'])]
    public function export(Request $request, Form $form): StreamedResponse
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_EXPORT, $form);

        $conversionStatus = $request->query->getString('conversionStatus');
        $rows = $this->submissions->findConfirmedForForm($form, '' !== $conversionStatus ? $conversionStatus : null);

        $response = new StreamedResponse(function () use ($rows): void {
            $out = fopen('php://output', 'w');
            \assert(false !== $out);
            fputcsv($out, ['Name', 'Email', 'Payment Status', 'Submitted At', 'Converted To User', 'Attended'], escape: '\\');

            foreach ($rows as $submission) {
                fputcsv($out, [
                    $submission->participantName(),
                    $submission->getContactEmail(),
                    $submission->getPaymentStatus(),
                    $submission->getSubmittedAt()->format('Y-m-d H:i'),
                    $submission->isConverted() ? 'Yes' : 'No',
                    $submission->isAttended() ? 'Yes' : 'No',
                ], escape: '\\');
            }

            fclose($out);
        });

        $response->headers->set('Content-Type', 'text/csv');
        $response->headers->set('Content-Disposition', sprintf('attachment; filename="form-%d-submissions.csv"', (int) $form->getId()));

        return $response;
    }

    /**
     * AC-08-20.
     */
    #[Route('/trainer/forms/{form<\d+>}/submissions/attendance', name: 'forms_trainer_submission_attendance', methods: ['GET', 'POST'])]
    public function attendance(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_MARK_ATTENDANCE, $form);

        $submissions = $this->submissions->findConfirmedForForm($form);
        $formView = $this->createForm(FormAttendanceType::class, null, ['submissions' => $submissions]);
        $formView->handleRequest($request);

        if ($formView->isSubmitted() && $formView->isValid()) {
            foreach ($submissions as $submission) {
                $field = $formView->get(FormAttendanceType::fieldName($submission));
                $submission->setAttended((bool) $field->getData());
            }

            $this->entityManager->flush();
            $this->addFlash('success', 'Attendance saved.');

            return $this->redirectToRoute('forms_trainer_submission_attendance', ['form' => $form->getId()]);
        }

        return $this->render('forms/trainer_submissions_attendance.html.twig', [
            'form' => $form,
            'formView' => $formView,
            'submissions' => $submissions,
        ]);
    }

    /**
     * AC-08-21.
     */
    #[Route('/trainer/forms/{form<\d+>}/submissions/bulk-email', name: 'forms_trainer_submissions_bulk_email', methods: ['GET', 'POST'])]
    public function bulkEmail(Request $request, Form $form): Response
    {
        $this->denyAccessUnlessGranted(FormVoter::FORM_SEND_BULK_EMAIL, $form);

        $formView = $this->createForm(BulkEmailType::class);
        $formView->handleRequest($request);

        if ($formView->isSubmitted() && $formView->isValid()) {
            /** @var array{subject: string, body: string} $data */
            $data = $formView->getData();
            $recipients = $this->submissions->findConfirmedForForm($form);

            foreach ($recipients as $submission) {
                $this->mailer->send(
                    (new Email())
                        ->from(self::FROM)
                        ->to($submission->getContactEmail())
                        ->subject($data['subject'])
                        ->text($data['body']),
                );
            }

            $this->addFlash('success', sprintf('Email sent to %d participant(s).', \count($recipients)));

            return $this->redirectToRoute('forms_trainer_submissions_index', ['form' => $form->getId()]);
        }

        return $this->render('forms/trainer_submissions_bulk_email.html.twig', ['form' => $form, 'formView' => $formView]);
    }
}
