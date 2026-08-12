<?php

declare(strict_types=1);

namespace App\Scheduling\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\AvailabilityWindowRepository;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\MembershipService;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use App\Scheduling\Dto\EventInput;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Exception\AlreadyRegisteredException;
use App\Scheduling\Exception\CapacityBelowRsvpCountException;
use App\Scheduling\Exception\CoachAssignmentConflictException;
use App\Scheduling\Exception\StripeNotConnectedException;
use App\Scheduling\Form\CancelEventType;
use App\Scheduling\Form\EventType;
use App\Scheduling\Form\ManualAddPlayerType;
use App\Scheduling\Form\RecurringEventType;
use App\Scheduling\Form\TakeAttendanceType;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\AttendanceService;
use App\Scheduling\Service\EventService;
use App\Scheduling\Service\RsvpService;
use App\Scheduling\Voter\AttendanceVoter;
use App\Scheduling\Voter\EventVoter;
use App\Scheduling\Voter\RsvpVoter;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.01..05, 12, 13, 14: Event Builder — create, view, edit, duplicate,
 * cancel, RSVP roster management, attendance.
 *
 * @see specs/api-designer-spec.md "Scheduling module" — trainer console table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerEventController extends AbstractController
{
    public function __construct(
        private readonly EventRepository $events,
        private readonly RsvpRepository $rsvps,
        private readonly AttendanceRecordRepository $attendanceRecords,
        private readonly EventService $eventService,
        private readonly RsvpService $rsvpService,
        private readonly AttendanceService $attendanceService,
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly MembershipService $membershipService,
        private readonly AvailabilityWindowRepository $availabilityWindows,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-02-1/US-02.01 narrative.
     */
    #[Route('/trainer/events', name: 'scheduling_trainer_event_index', methods: ['GET'])]
    public function index(): Response
    {
        return $this->render('scheduling/trainer_event_index.html.twig', [
            'events' => $this->events->findAllForActiveTenant(),
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-02-1..3, BR-02-1..4.
     */
    #[Route('/trainer/events/new', name: 'scheduling_trainer_event_create', methods: ['GET', 'POST'])]
    public function create(Request $request): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_CREATE);

        $trainer = $this->currentTrainer();
        $form = $this->createForm(EventType::class, $this->defaultData(), $this->formOptions($trainer, 'Create event'));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            try {
                $event = $this->eventService->create($trainer, $this->actor(), $this->toEventInput($data));
            } catch (CoachAssignmentConflictException $e) {
                $form->get('coach')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'create']);
            } catch (StripeNotConnectedException $e) {
                // AC-05-3: "Connect Stripe first" — a usd-priced event was
                // attempted with no Connect account.
                $form->get('usdPricingEnabled')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'create']);
            }

            $this->addFlash('success', 'Event created successfully.');

            return $this->redirectToRoute('scheduling_trainer_event_show', ['event' => $event->getId()]);
        }

        return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'create']);
    }

    /**
     * AC-02-61: "Every Tuesday for 3 months" — one submitted form
     * bulk-generates a weekly recurring pattern; see
     * EventService::createRecurring()'s own docblock for exactly how the
     * occurrences are generated and why they are fully independent
     * afterward.
     */
    #[Route('/trainer/events/new-recurring', name: 'scheduling_trainer_event_create_recurring', methods: ['GET', 'POST'])]
    public function createRecurring(Request $request): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_CREATE);

        $trainer = $this->currentTrainer();
        $form = $this->createForm(RecurringEventType::class, $this->defaultData(), $this->formOptions($trainer, 'Create recurring events'));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();
            /** @var \DateTimeImmutable $repeatUntil */
            $repeatUntil = $data['repeatUntil'];

            try {
                $created = $this->eventService->createRecurring($trainer, $this->actor(), $this->toEventInput($data), $repeatUntil);
            } catch (CoachAssignmentConflictException $e) {
                $form->get('coach')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_recurring_form.html.twig', ['form' => $form]);
            } catch (\InvalidArgumentException $e) {
                $form->get('repeatUntil')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_recurring_form.html.twig', ['form' => $form]);
            } catch (StripeNotConnectedException $e) {
                $form->get('usdPricingEnabled')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_recurring_form.html.twig', ['form' => $form]);
            }

            $this->addFlash('success', sprintf('%d events created.', \count($created)));

            return $this->redirectToRoute('scheduling_trainer_event_index');
        }

        return $this->render('scheduling/trainer_event_recurring_form.html.twig', ['form' => $form]);
    }

    /**
     * AC-02-12: "15 of 20 eligible players available at this time."
     */
    #[Route('/trainer/events/availability-check', name: 'scheduling_trainer_event_availability_check', methods: ['GET'])]
    public function availabilityCheck(Request $request): Response
    {
        $trainer = $this->currentTrainer();
        $at = new \DateTimeImmutable((string) $request->query->get('start', 'now'));

        return $this->json($this->eventService->availabilityCount($trainer, $at));
    }

    #[Route('/trainer/events/{event<\d+>}', name: 'scheduling_trainer_event_show', methods: ['GET'])]
    public function show(Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_VIEW, $event);

        return $this->render('scheduling/trainer_event_show.html.twig', [
            'event' => $event,
            'now' => new \DateTimeImmutable(),
            'rsvpCount' => $this->rsvps->countHeld($event),
            // AC-02-40: recorded attendance is visible to the trainer in
            // the event's own details.
            'attendanceRecords' => $this->attendanceRecords->findForEvent($event),
        ]);
    }

    /**
     * AC-02-50..54.
     */
    #[Route('/trainer/events/{event<\d+>}/edit', name: 'scheduling_trainer_event_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_EDIT, $event);

        // AC-02-54: events in the past, or canceled, cannot be edited —
        // checked before ever rendering the form (GET) or processing it
        // (POST), same reasoning as cancel()'s own early check just above:
        // EventService::update()'s own guard is a precondition on the
        // event itself, and an uncaught \LogicException from there would
        // otherwise surface as a bare 500.
        if (!$event->isEditable(new \DateTimeImmutable())) {
            $this->addFlash('error', 'This event can no longer be edited — it is canceled or has already started.');

            return $this->redirectToRoute('scheduling_trainer_event_show', ['event' => $event->getId()]);
        }

        $trainer = $event->getTrainer();
        $form = $this->createForm(EventType::class, $this->dataFromEvent($event), $this->formOptions($trainer, 'Save changes', skipPastCheck: true));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            try {
                $this->eventService->update($event, $this->actor(), $this->toEventInput($data));
            } catch (CapacityBelowRsvpCountException $e) {
                $form->get('capacity')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'edit', 'event' => $event]);
            } catch (CoachAssignmentConflictException $e) {
                $form->get('coach')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'edit', 'event' => $event]);
            }

            $this->addFlash('success', 'Event updated.');

            return $this->redirectToRoute('scheduling_trainer_event_show', ['event' => $event->getId()]);
        }

        return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'edit', 'event' => $event]);
    }

    /**
     * AC-02-14..17.
     */
    #[Route('/trainer/events/{event<\d+>}/duplicate', name: 'scheduling_trainer_event_duplicate', methods: ['GET', 'POST'])]
    public function duplicate(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_DUPLICATE, $event);

        $trainer = $event->getTrainer();
        // AC-02-15: pre-filled, but the date/time must be changed — leaving
        // it as-is is exactly the input EventService::duplicate() rejects.
        $form = $this->createForm(EventType::class, $this->dataFromEvent($event), $this->formOptions($trainer, 'Create duplicate'));
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            try {
                $new = $this->eventService->duplicate($event, $this->actor(), $this->toEventInput($data));
            } catch (\InvalidArgumentException $e) {
                $form->get('startsAt')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'duplicate', 'event' => $event]);
            } catch (CoachAssignmentConflictException $e) {
                $form->get('coach')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'duplicate', 'event' => $event]);
            } catch (StripeNotConnectedException $e) {
                $form->get('usdPricingEnabled')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'duplicate', 'event' => $event]);
            }

            $this->addFlash('success', 'Event duplicated.');

            return $this->redirectToRoute('scheduling_trainer_event_show', ['event' => $new->getId()]);
        }

        return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'duplicate', 'event' => $event]);
    }

    /**
     * AC-02-46..49.
     */
    #[Route('/trainer/events/{event<\d+>}/cancel', name: 'scheduling_trainer_event_cancel', methods: ['GET', 'POST'])]
    public function cancel(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_CANCEL, $event);

        // AC-02-49: an already-started or completed event cannot be
        // canceled — checked before ever rendering the form (GET) or
        // processing it (POST), since EventService::cancel()'s own guard
        // is a precondition on the event itself, not something a
        // corrected form resubmission could ever satisfy. Without this,
        // an uncaught \LogicException from the service would surface as a
        // bare 500 instead of a clean, expected redirect.
        if (!$event->isCancelable(new \DateTimeImmutable())) {
            $this->addFlash('error', 'This event can no longer be canceled — it has already started or completed.');

            return $this->redirectToRoute('scheduling_trainer_event_show', ['event' => $event->getId()]);
        }

        $form = $this->createForm(CancelEventType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{reason: string} $data */
            $data = $form->getData();

            $this->eventService->cancel($event, $data['reason'], $this->actor());
            $this->addFlash('success', 'Event canceled. Registered players have been notified.');

            return $this->redirectToRoute('scheduling_trainer_event_index');
        }

        return $this->render('scheduling/trainer_event_cancel.html.twig', ['form' => $form, 'event' => $event]);
    }

    /**
     * AC-02-43/44.
     */
    #[Route('/trainer/events/{event<\d+>}/rsvps', name: 'scheduling_trainer_event_rsvps', methods: ['GET'])]
    public function rsvps(Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_VIEW_RSVP_LIST, $event);

        return $this->render('scheduling/trainer_event_rsvps.html.twig', [
            'event' => $event,
            'rsvpRows' => $this->rsvpRows($event),
        ]);
    }

    /**
     * AC-02-45.
     */
    #[Route('/trainer/events/{event<\d+>}/rsvps/export', name: 'scheduling_trainer_event_rsvps_export', methods: ['GET'])]
    public function rsvpsExport(Event $event): StreamedResponse
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_EXPORT_RSVPS, $event);

        $response = new StreamedResponse(function () use ($event): void {
            $out = fopen('php://output', 'w');
            \assert(false !== $out);
            fputcsv($out, ['Player', 'Age', 'Skill Level', 'RSVP Time', 'Payment Status'], escape: '\\');

            foreach ($this->rsvpRows($event) as $row) {
                fputcsv($out, [$row['name'], $row['age'], $row['skillLevel'], $row['requestedAt'], $row['paymentStatus']], escape: '\\');
            }

            fclose($out);
        });

        $response->headers->set('Content-Type', 'text/csv');
        $response->headers->set('Content-Disposition', sprintf('attachment; filename="event-%d-rsvps.csv"', (int) $event->getId()));

        return $response;
    }

    /**
     * AC-02-45/Q-02.09.
     */
    #[Route('/trainer/events/{event<\d+>}/rsvps/add', name: 'scheduling_trainer_event_rsvp_add', methods: ['GET', 'POST'])]
    public function rsvpAdd(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_MANUAL_ADD_PLAYER, $event);

        $candidates = $this->candidatePlayers($event);
        $form = $this->createForm(ManualAddPlayerType::class, null, ['candidates' => $candidates]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{player: PlayerProfile} $data */
            $data = $form->getData();

            try {
                $this->rsvpService->manuallyAdd($event, $data['player'], $this->actor());
            } catch (AlreadyRegisteredException $e) {
                $form->get('player')->addError(new FormError($e->getMessage()));

                return $this->render('scheduling/trainer_event_rsvp_add.html.twig', ['form' => $form, 'event' => $event]);
            }

            $this->addFlash('success', 'Player added to the event.');

            return $this->redirectToRoute('scheduling_trainer_event_rsvps', ['event' => $event->getId()]);
        }

        return $this->render('scheduling/trainer_event_rsvp_add.html.twig', ['form' => $form, 'event' => $event]);
    }

    /**
     * AC-02-45.
     */
    #[Route('/trainer/events/{event<\d+>}/rsvps/{rsvp<\d+>}/remove', name: 'scheduling_trainer_event_rsvp_remove', methods: ['POST'])]
    public function rsvpRemove(Request $request, Event $event, Rsvp $rsvp): Response
    {
        $this->denyAccessUnlessGranted(RsvpVoter::RSVP_REMOVE, $rsvp);

        if (!$this->isCsrfTokenValid('rsvp-remove'.$rsvp->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->rsvpService->remove($rsvp, $this->actor());
        $this->addFlash('success', 'Player removed from the event.');

        return $this->redirectToRoute('scheduling_trainer_event_rsvps', ['event' => $event->getId()]);
    }

    /**
     * AC-02-41/BR-02-18: trainer branch — any time, overrides the coach.
     */
    #[Route('/trainer/events/{event<\d+>}/attendance', name: 'scheduling_trainer_event_attendance', methods: ['GET', 'POST'])]
    public function attendance(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(AttendanceVoter::ATTENDANCE_RECORD, $event);

        return $this->handleAttendanceForm($request, $event, 'scheduling_trainer_event_attendance');
    }

    private function handleAttendanceForm(Request $request, Event $event, string $redirectRoute): Response
    {
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

            return $this->redirectToRoute($redirectRoute, ['event' => $event->getId()]);
        }

        return $this->render('scheduling/attendance_take.html.twig', ['form' => $form, 'event' => $event, 'players' => $players]);
    }

    /**
     * @return list<array{rsvpId: int, name: string, age: int, skillLevel: string, requestedAt: string, paymentStatus: string, availability: string}>
     */
    private function rsvpRows(Event $event): array
    {
        $rows = [];

        foreach ($this->rsvps->findForEvent($event) as $rsvp) {
            $player = $rsvp->getPlayer();
            $membership = $this->memberships->findOneByTrainerAndPlayer($event->getTrainer(), $player);

            $rows[] = [
                'rsvpId' => (int) $rsvp->getId(),
                'name' => $player->getFirstName(),
                'age' => $player->ageOn($event->getStartsAt()),
                'skillLevel' => $membership?->getSkillLevel() ?? '',
                'requestedAt' => $rsvp->getRequestedAt()->setTimezone($event->getTrainer()->getTimezone())->format('Y-m-d H:i'),
                'paymentStatus' => $this->paymentStatusLabel($rsvp),
                'availability' => $this->availabilityMatch($event, $player),
            ];
        }

        return $rows;
    }

    private function paymentStatusLabel(Rsvp $rsvp): string
    {
        if (Rsvp::METHOD_FREE === $rsvp->getPaymentMethod()) {
            return 'Free';
        }

        return match ($rsvp->getStatus()) {
            Rsvp::STATUS_CONFIRMED => 'Paid',
            Rsvp::STATUS_CANCELED => 'Refunded',
            default => 'Pending',
        };
    }

    /**
     * AC-02-43/BR-02-20: green if the event is fully within a declared
     * *available* window for that player, red if it conflicts with a
     * window the player marked unavailable, gray if the player has stated
     * nothing for that day/time. Shown to the trainer only — BR-02-20 is
     * explicit that this is never surfaced to the player.
     */
    private function availabilityMatch(Event $event, PlayerProfile $player): string
    {
        // Normalized to PHP's own runtime-default timezone, NOT the
        // trainer's — see CoachAssignmentService::guardNoConflict()'s own
        // comment: AvailabilityWindow (Epic-01) carries no timezone of its
        // own, so comparing it against a trainer-local-converted event time
        // would compare two different zones.
        $utc = new \DateTimeZone(date_default_timezone_get());
        $normalizedStart = $event->getStartsAt()->setTimezone($utc);
        $normalizedEnd = $event->getEndsAt()->setTimezone($utc);
        $dayOfWeek = (int) $normalizedStart->format('w');
        $timeStart = new \DateTimeImmutable('1970-01-01 '.$normalizedStart->format('H:i:s'));
        $timeEnd = new \DateTimeImmutable('1970-01-01 '.$normalizedEnd->format('H:i:s'));

        $windows = $this->availabilityWindows->findForPlayer($player);

        foreach ($windows as $window) {
            if ($window->isAvailable()
                && $window->getDayOfWeek() === $dayOfWeek
                && $window->getStartTime() <= $timeStart
                && $window->getEndTime() >= $timeEnd
            ) {
                return 'green';
            }
        }

        foreach ($windows as $window) {
            if (!$window->isAvailable() && $window->overlaps($dayOfWeek, $timeStart, $timeEnd)) {
                return 'red';
            }
        }

        return 'gray';
    }

    /**
     * @return array<int, PlayerProfile>
     */
    private function candidatePlayers(Event $event): array
    {
        $registered = array_map(
            static fn (Rsvp $r): int => (int) $r->getPlayer()->getId(),
            $this->rsvps->findForEvent($event),
        );

        $candidates = [];
        foreach ($this->memberships->findActiveForActiveTenant() as $membership) {
            $player = $membership->getPlayer();
            $id = (int) $player->getId();

            if (!\in_array($id, $registered, true)) {
                $candidates[$id] = $player;
            }
        }

        return $candidates;
    }

    /**
     * @return array<int, PlayerProfile>
     */
    private function invitablePlayers(Trainer $trainer): array
    {
        $candidates = [];
        foreach ($this->memberships->findActiveForActiveTenant() as $membership) {
            $player = $membership->getPlayer();
            $candidates[(int) $player->getId()] = $player;
        }

        return $candidates;
    }

    /**
     * @return array<int, CoachMembership>
     */
    private function assignableCoaches(Trainer $trainer): array
    {
        // AC-02-8: the dropdown always includes the trainer themselves.
        $self = $this->membershipService->ensureSelfCoachMembership($trainer);

        $coaches = [(int) $self->getId() => $self];

        foreach ($this->coachMemberships->findAllForActiveTenant() as $membership) {
            if ($membership->isActive()) {
                $coaches[(int) $membership->getId()] = $membership;
            }
        }

        return $coaches;
    }

    /**
     * @return array<string, mixed>
     */
    private function formOptions(Trainer $trainer, string $submitLabel, bool $skipPastCheck = false): array
    {
        return [
            'timezone' => $trainer->getTimezone(),
            'invitablePlayers' => $this->invitablePlayers($trainer),
            'assignableCoaches' => $this->assignableCoaches($trainer),
            'submitLabel' => $submitLabel,
            'skipPastCheck' => $skipPastCheck,
        ];
    }

    /**
     * @return array<string, mixed>
     */
    private function defaultData(): array
    {
        return [
            'eventType' => Event::TYPE_TRAINING_SESSION,
            'visibility' => Event::VISIBILITY_PUBLIC,
            'capacity' => 10,
            'usdPricingEnabled' => false,
            'usdPrice' => 0,
            'tokenPricingEnabled' => true,
            'tokenPrice' => 1,
        ];
    }

    /**
     * The "coach" field is deliberately left unselected here even when
     * editing an event that already has one: EventType's coach field is
     * "reassign to" input, not a display of the current assignment (shown
     * separately on the template), and AC-02-51's change-notification logic
     * lives in EventService's own diff against $before, not in what this
     * form pre-fills.
     *
     * @return array<string, mixed>
     */
    private function dataFromEvent(Event $event): array
    {
        // EventType's DateTimeType fields are configured with
        // model_timezone = the trainer's own zone (see formOptions()); the
        // form component requires the pre-filled DateTimeImmutable's own
        // timezone to already match that exactly, or it throws rather than
        // converting. getStartsAt()/getEndsAt() come back from Doctrine
        // carrying whatever offset Postgres reported (UTC), so this must
        // convert explicitly before handing the value to the form.
        $tz = $event->getTrainer()->getTimezone();

        return [
            'title' => $event->getTitle(),
            'eventType' => $event->getEventType(),
            'startsAt' => $event->getStartsAt()->setTimezone($tz),
            'endsAt' => $event->getEndsAt()->setTimezone($tz),
            'location' => $event->getLocation(),
            'capacity' => $event->getCapacity(),
            'visibility' => $event->getVisibility(),
            'description' => $event->getDescription(),
            'minAge' => $event->getMinAge(),
            'maxAge' => $event->getMaxAge(),
            'skillLevels' => $event->getSkillLevels() ?? [],
            'genders' => $event->getGenders() ?? [],
            'usdPricingEnabled' => $event->isUsdPricingEnabled(),
            'usdPrice' => $event->getUsdPriceMinorUnits() / 100,
            'tokenPricingEnabled' => $event->isTokenPricingEnabled(),
            'tokenPrice' => $event->getTokenPrice(),
        ];
    }

    /**
     * @param array<string, mixed> $data
     */
    private function toEventInput(array $data): EventInput
    {
        $skillLevels = $this->selectedList($data['skillLevels'] ?? null);
        $genders = $this->selectedList($data['genders'] ?? null);

        // EntityType with 'multiple' => true submits a Doctrine Collection
        // (ArrayCollection), even on this array-mapped (no data_class) form
        // — never a plain PHP array — so array_map() needs it normalized
        // first via iterator_to_array().
        $invitedPlayersRaw = $data['invitedPlayers'] ?? [];
        /** @var list<PlayerProfile> $invitedPlayerEntities */
        $invitedPlayerEntities = $invitedPlayersRaw instanceof \Traversable ? iterator_to_array($invitedPlayersRaw, false) : $invitedPlayersRaw;
        $invitedPlayerIds = array_map(static fn (PlayerProfile $p): int => (int) $p->getId(), $invitedPlayerEntities);

        /** @var CoachMembership|null $coach */
        $coach = $data['coach'] ?? null;

        return new EventInput(
            title: (string) $data['title'],
            eventType: (string) $data['eventType'],
            startsAt: $data['startsAt'],
            endsAt: $data['endsAt'],
            location: (string) $data['location'],
            capacity: (int) $data['capacity'],
            visibility: (string) $data['visibility'],
            description: '' === ($data['description'] ?? '') ? null : $data['description'],
            minAge: isset($data['minAge']) ? (int) $data['minAge'] : null,
            maxAge: isset($data['maxAge']) ? (int) $data['maxAge'] : null,
            skillLevels: $skillLevels,
            genders: $genders,
            usdPricingEnabled: (bool) ($data['usdPricingEnabled'] ?? false),
            usdPriceMinorUnits: (int) round((float) ($data['usdPrice'] ?? 0) * 100),
            tokenPricingEnabled: (bool) ($data['tokenPricingEnabled'] ?? false),
            tokenPrice: max(1, (int) ($data['tokenPrice'] ?? 1)),
            invitedPlayerIds: $invitedPlayerIds,
            coachMembershipId: $coach?->getId(),
            coachOverrideReason: '' === ($data['coachOverrideReason'] ?? '') ? null : $data['coachOverrideReason'],
        );
    }

    /**
     * Both eligibility axes are `multiple` ChoiceTypes now, so each arrives
     * as a list rather than a comma-separated string. An empty selection
     * means "no restriction", which the entity stores as null rather than an
     * empty array.
     *
     * @return list<string>|null
     */
    private function selectedList(mixed $raw): ?array
    {
        if (!\is_array($raw) || [] === $raw) {
            return null;
        }

        return array_values(array_map(strval(...), $raw));
    }

    private function currentTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
