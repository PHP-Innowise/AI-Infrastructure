<?php

declare(strict_types=1);

namespace App\Scheduling\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Form\DeclineAssignmentType;
use App\Scheduling\Form\TakeAttendanceType;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\AttendanceService;
use App\Scheduling\Service\CoachAssignmentService;
use App\Scheduling\Voter\AttendanceVoter;
use App\Scheduling\Voter\CoachAssignmentVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.10/11: "My Activities" — pending assignments to confirm/decline, and
 * attendance for assigned, started events.
 *
 * @see specs/api-designer-spec.md "Scheduling module" — coach console table
 */
#[IsGranted('ROLE_COACH')]
final class CoachActivityController extends AbstractController
{
    public function __construct(
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly CoachAssignmentRepository $assignments,
        private readonly CoachAssignmentService $coachAssignmentService,
        private readonly AttendanceService $attendanceService,
        private readonly RsvpRepository $rsvps,
        private readonly AttendanceRecordRepository $attendanceRecords,
    ) {
    }

    /**
     * AC-02-34: "Events to Confirm" + "Assigned Sessions" — the query
     * itself is scoped to the acting coach (specs/api-designer-spec.md:486,
     * "— (scoped by CoachVisibilityService)"), no voter needed for the list
     * itself. Each "Events to Confirm" row additionally carries the number
     * of players RSVP'd (AC-02-34's own wording), keyed by assignment id
     * since a template cannot call RsvpRepository directly.
     */
    #[Route('/coach/activities', name: 'scheduling_coach_activities', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $membership = $this->currentMembership();
        $tab = (string) $request->query->get('tab', 'to-confirm');
        $toConfirm = $this->assignments->findPendingForCoach($membership);

        $rsvpCounts = [];
        foreach ($toConfirm as $assignment) {
            $rsvpCounts[(int) $assignment->getId()] = $this->rsvps->countHeld($assignment->getEvent());
        }

        return $this->render('scheduling/coach_activities.html.twig', [
            'tab' => $tab,
            'toConfirm' => $toConfirm,
            'rsvpCounts' => $rsvpCounts,
            'assigned' => $this->assignments->findConfirmedForCoach($membership),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-02-35.
     */
    #[Route('/coach/assignments/{assignment<\d+>}/confirm', name: 'scheduling_coach_assignment_confirm', methods: ['POST'])]
    public function confirm(Request $request, CoachAssignment $assignment): Response
    {
        $this->denyAccessUnlessGranted(CoachAssignmentVoter::COACH_ASSIGNMENT_CONFIRM, $assignment);

        if (!$this->isCsrfTokenValid('assignment-confirm'.$assignment->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->coachAssignmentService->confirm($assignment);
        $this->addFlash('success', 'Assignment confirmed.');

        return $this->redirectToRoute('scheduling_coach_activities');
    }

    /**
     * AC-02-36.
     */
    #[Route('/coach/assignments/{assignment<\d+>}/decline', name: 'scheduling_coach_assignment_decline', methods: ['GET', 'POST'])]
    public function decline(Request $request, CoachAssignment $assignment): Response
    {
        $this->denyAccessUnlessGranted(CoachAssignmentVoter::COACH_ASSIGNMENT_DECLINE, $assignment);

        $form = $this->createForm(DeclineAssignmentType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{reason: ?string} $data */
            $data = $form->getData();
            $this->coachAssignmentService->decline($assignment, $data['reason']);
            $this->addFlash('success', 'Assignment declined.');

            return $this->redirectToRoute('scheduling_coach_activities');
        }

        return $this->render('scheduling/coach_assignment_decline.html.twig', ['form' => $form, 'assignment' => $assignment]);
    }

    /**
     * AC-02-38..42/BR-02-16..18: coach branch — assigned, started,
     * same-day-only.
     */
    #[Route('/coach/events/{event<\d+>}/attendance', name: 'scheduling_coach_event_attendance', methods: ['GET', 'POST'])]
    public function attendance(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(AttendanceVoter::ATTENDANCE_RECORD, $event);

        $players = [];
        $current = [];

        foreach ($this->rsvps->findForEvent($event) as $rsvp) {
            if (Rsvp::STATUS_CONFIRMED !== $rsvp->getStatus()) {
                continue;
            }

            $playerId = (int) $rsvp->getPlayer()->getId();
            $players[$playerId] = $rsvp->getPlayer()->getFirstName();

            $record = $this->attendanceRecords->findOneByEventAndPlayer($event, $rsvp->getPlayer());
            if (null !== $record) {
                $current[$playerId] = $record->getStatus();
            }
        }

        $form = $this->createForm(TakeAttendanceType::class, null, ['players' => $players, 'current' => $current]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, string> $data */
            $data = $form->getData();
            $statusByPlayerId = [];

            foreach ($data as $key => $status) {
                if (str_starts_with($key, 'player_')) {
                    $statusByPlayerId[(int) substr($key, 7)] = $status;
                }
            }

            $this->attendanceService->recordAttendance($event, $this->actor(), $statusByPlayerId);
            $this->addFlash('success', 'Attendance saved.');

            return $this->redirectToRoute('scheduling_coach_event_attendance', ['event' => $event->getId()]);
        }

        return $this->render('scheduling/attendance_take.html.twig', ['form' => $form, 'event' => $event, 'players' => $players]);
    }

    private function currentMembership(): CoachMembership
    {
        $membership = $this->coachMemberships->findOneForAccountInActiveTenant($this->actor());

        return $membership ?? throw $this->createNotFoundException('No coach membership for this account in the active tenant.');
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
