<?php

declare(strict_types=1);

namespace App\Crm\Controller;

use App\Crm\Entity\PlayerNote;
use App\Crm\Exception\NoteEditWindowExpiredException;
use App\Crm\Form\SessionFeedbackType;
use App\Crm\Repository\PlayerFlagRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Crm\Repository\PlayerNoteRepository;
use App\Crm\Service\PlayerNoteService;
use App\Crm\Voter\PlayerVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Service\CoachVisibilityService;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-03.09, US-03.10: the coach's own, event-scoped player list, detail,
 * and session feedback.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-43..49, AC-03-64/65
 */
#[IsGranted('ROLE_COACH')]
final class CoachPlayerController extends AbstractController
{
    public function __construct(
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly CoachVisibilityService $coachVisibility,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly AttendanceRecordRepository $attendanceRecords,
        private readonly PlayerLabelRepository $playerLabels,
        private readonly PlayerFlagRepository $playerFlags,
        private readonly PlayerNoteRepository $playerNotes,
        private readonly PlayerNoteService $playerNoteService,
    ) {
    }

    /**
     * AC-03-43/64/65: only players RSVP'd to this coach's own assigned
     * events — never the trainer's full roster, and search never reaches
     * beyond this set.
     */
    #[Route('/coach/players', name: 'crm_coach_players_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $coach = $this->currentCoachMembership();
        $memberships = $this->memberships->findActiveForPlayerIds($this->coachVisibility->reachablePlayerIds($coach));

        $q = $request->query->get('q');

        if (null !== $q && '' !== trim((string) $q)) {
            $needle = mb_strtolower((string) $q);
            $memberships = array_values(array_filter(
                $memberships,
                static fn (PlayerTrainerMembership $m): bool => str_contains(mb_strtolower($m->getPlayer()->getFirstName()), $needle),
            ));
        }

        $rows = array_map(
            fn (PlayerTrainerMembership $m): array => [
                'membership' => $m,
                'sessionsWithCoach' => \count($this->attendanceRecords->findForPlayerAndCoach($m->getPlayer(), $coach)),
            ],
            $memberships,
        );

        return $this->render('crm/coach_players_index.html.twig', ['rows' => $rows]);
    }

    /**
     * AC-03-44/45: coach-scoped detail — attendance/labels/flags/feedback
     * limited to this coach's own shared history, all read-only except
     * feedback.
     */
    #[Route('/coach/players/{membership}', name: 'crm_coach_player_show', requirements: ['membership' => '\d+'], methods: ['GET'])]
    public function show(PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_VIEW, $membership);

        $coach = $this->currentCoachMembership();
        $player = $membership->getPlayer();
        $records = $this->attendanceRecords->findForPlayerAndCoach($player, $coach);

        $attended = \count(array_filter(
            $records,
            static fn (AttendanceRecord $r): bool => \in_array($r->getStatus(), [AttendanceRecord::STATUS_PRESENT, AttendanceRecord::STATUS_LATE], true),
        ));

        return $this->render('crm/coach_player_show.html.twig', [
            'now' => new \DateTimeImmutable(),
            'membership' => $membership,
            'player' => $player,
            'attendanceRecords' => $records,
            'attendedCount' => $attended,
            'totalCount' => \count($records),
            'labels' => $this->playerLabels->findForPlayer($player),
            'activeFlags' => $this->playerFlags->findActiveForPlayer($player),
            'pastFeedback' => $this->playerNotes->findSessionNotesForPlayerByAuthor($player, $this->currentAccount()),
            'feedbackForm' => $this->createForm(SessionFeedbackType::class, null, ['recentSessions' => $this->recentSessionChoices($records)]),
        ]);
    }

    /**
     * AC-03-46: up to 500 characters, tied to one of this coach's own recent
     * shared sessions with the player.
     */
    #[Route('/coach/players/{membership}/feedback', name: 'crm_coach_player_feedback_add', methods: ['POST'])]
    public function feedbackAdd(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FEEDBACK_ADD, $membership);

        $coach = $this->currentCoachMembership();
        $records = $this->attendanceRecords->findForPlayerAndCoach($membership->getPlayer(), $coach);
        $form = $this->createForm(SessionFeedbackType::class, null, ['recentSessions' => $this->recentSessionChoices($records)]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{event: Event, text: string} $data */
            $data = $form->getData();
            $this->playerNoteService->addSessionNote($membership, $data['event'], $this->currentAccount(), $data['text']);
            $this->addFlash('success', 'Feedback added.');
        }

        return $this->redirectToRoute('crm_coach_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * AC-03-49: own feedback only, within 24 hours.
     */
    #[Route('/coach/players/{membership}/feedback/{feedback}/edit', name: 'crm_coach_player_feedback_edit', methods: ['POST'])]
    public function feedbackEdit(Request $request, PlayerTrainerMembership $membership, PlayerNote $feedback): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FEEDBACK_EDIT, $feedback);

        $coach = $this->currentCoachMembership();
        $records = $this->attendanceRecords->findForPlayerAndCoach($membership->getPlayer(), $coach);
        $form = $this->createForm(SessionFeedbackType::class, null, ['recentSessions' => $this->recentSessionChoices($records)]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{text: string} $data */
            $data = $form->getData();

            try {
                $this->playerNoteService->edit($feedback, $this->currentAccount(), $data['text'], new \DateTimeImmutable());
                $this->addFlash('success', 'Feedback updated.');
            } catch (NoteEditWindowExpiredException $e) {
                $this->addFlash('error', $e->getMessage());
            }
        }

        return $this->redirectToRoute('crm_coach_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * AC-03-49: "Coach can edit or delete their own feedback within 24
     * hours" — this route completes api-designer-spec's route table, which
     * draws the edit action but not a matching delete one, following the
     * SAME shape it already uses for the trainer's own note delete
     * (`crm_trainer_player_note_delete`). Recorded in the coder's final
     * report.
     */
    #[Route('/coach/players/{membership}/feedback/{feedback}/delete', name: 'crm_coach_player_feedback_delete', methods: ['POST'])]
    public function feedbackDelete(Request $request, PlayerTrainerMembership $membership, PlayerNote $feedback): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FEEDBACK_EDIT, $feedback);

        if (!$this->isCsrfTokenValid('coach-feedback-delete'.$feedback->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->playerNoteService->delete($feedback);
        $this->addFlash('success', 'Feedback deleted.');

        return $this->redirectToRoute('crm_coach_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * @param list<AttendanceRecord> $records
     *
     * @return array<int, Event>
     */
    private function recentSessionChoices(array $records): array
    {
        $choices = [];

        foreach ($records as $record) {
            $event = $record->getEvent();
            $choices[(int) $event->getId()] = $event;
        }

        return $choices;
    }

    private function currentCoachMembership(): CoachMembership
    {
        $coach = $this->coachMemberships->findOneForAccountInActiveTenant($this->currentAccount());

        if (null === $coach) {
            throw $this->createAccessDeniedException('No active coach membership in this tenant.');
        }

        return $coach;
    }

    private function currentAccount(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
