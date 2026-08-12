<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\CoachMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\CrossTenantReadService;
use App\Platform\Tenancy\AdministrativeScope;
use App\Scheduling\Dto\EventInput;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Form\CancelEventType;
use App\Scheduling\Form\EventType;
use App\Scheduling\Repository\AttendanceRecordRepository;
use App\Scheduling\Repository\CoachAssignmentRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\EventService;
use App\Scheduling\Voter\EventVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.15: "Event Master Tool" — Super Admin views, edits and cancels any
 * trainer's events. AC-02-55/56/57.
 *
 * A minimal slice, not the full Epic-07 Administration console — see
 * AdministrativeScope's and CrossTenantReadService's own docblocks for why
 * both first land here, in Epic-02, scoped strictly to what these three ACs
 * need. Recorded as a cross-epic boundary in the coder's final report:
 * US-02.15 is textually inside the Epic-02 spec, but architect-architecture.md
 * and specs/api-designer-spec.md both structurally assign "Event Master
 * tool" to the `Administration` module (Epic-07) — this controller lives
 * there, calling Scheduling's own EventService, exactly as
 * specs/api-designer-spec.md's Administration module note states
 * ("May call every module's services; must never read another module's
 * repositories directly").
 *
 * @see specs/api-designer-spec.md "Administration module"
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class EventMasterController extends AbstractController
{
    public function __construct(
        private readonly CrossTenantReadService $crossTenantReads,
        private readonly TrainerRepository $trainers,
        private readonly AdministrativeScope $administrativeScope,
        private readonly EventRepository $events,
        private readonly RsvpRepository $rsvps,
        private readonly EventService $eventService,
        private readonly AttendanceRecordRepository $attendanceRecords,
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly CoachAssignmentRepository $coachAssignments,
    ) {
    }

    /**
     * AC-07-22..24: search + filters (trainer, location, date range, type,
     * status). AC-07-26: sort (date/trainer/capacity) + 50/page.
     */
    #[Route('/super-admin/events', name: 'administration_event_master_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $trainerId = $request->query->get('trainer');
        $dateFrom = $request->query->get('dateFrom');
        $dateTo = $request->query->get('dateTo');

        $result = $this->crossTenantReads->listEvents(
            actor: $this->actor(),
            query: $this->stringOrNull($request->query->get('q')),
            trainerId: null !== $trainerId && '' !== $trainerId ? (int) $trainerId : null,
            location: $this->stringOrNull($request->query->get('location')),
            dateFrom: null !== $dateFrom && '' !== $dateFrom ? new \DateTimeImmutable((string) $dateFrom) : null,
            dateTo: null !== $dateTo && '' !== $dateTo ? new \DateTimeImmutable((string) $dateTo) : null,
            eventType: $this->stringOrNull($request->query->get('type')),
            status: $this->stringOrNull($request->query->get('status')),
            page: max(1, (int) $request->query->get('page', 1)),
            sort: $this->stringOrNull($request->query->get('sort')),
        );

        return $this->render('administration/event_master_index.html.twig', [
            'items' => $result['items'],
            'total' => $result['total'],
            'page' => max(1, (int) $request->query->get('page', 1)),
            'perPage' => 50,
            'sort' => $this->stringOrNull($request->query->get('sort')) ?? 'date_desc',
        ]);
    }

    /**
     * AC-02-55/56.
     */
    #[Route('/super-admin/events/export', name: 'administration_event_master_export', methods: ['GET'])]
    public function export(): StreamedResponse
    {
        $result = $this->crossTenantReads->listEvents($this->actor(), null, null, null, null, null, null, null, 1, 100000);

        $response = new StreamedResponse(function () use ($result): void {
            $out = fopen('php://output', 'w');
            \assert(false !== $out);
            fputcsv($out, ['Title', 'Trainer', 'Type', 'Starts At', 'Location', 'Status'], escape: '\\');

            /** @var list<array<string, mixed>> $items */
            $items = $result['items'];
            foreach ($items as $row) {
                fputcsv($out, [$row['title'], $row['trainer_name'], $row['event_type'], $row['starts_at'], $row['location'], $row['display_status']], escape: '\\');
            }

            fclose($out);
        });

        $response->headers->set('Content-Type', 'text/csv');
        $response->headers->set('Content-Disposition', 'attachment; filename="events.csv"');

        return $response;
    }

    /**
     * AC-02-57: opens the scope for the event's own trainer to load the
     * real, managed entity (rather than a CrossTenantReadService
     * projection) — EventVoter::EVENT_VIEW then grants via
     * AdministrativeScope::isOpenFor().
     */
    #[Route('/super-admin/events/{event<\d+>}', name: 'administration_event_master_show', methods: ['GET'])]
    public function show(int $event): Response
    {
        return $this->withScope($event, function (Event $eventEntity): Response {
            $this->denyAccessUnlessGranted(EventVoter::EVENT_VIEW, $eventEntity);

            return $this->render('scheduling/trainer_event_show.html.twig', [
                'event' => $eventEntity,
                'now' => new \DateTimeImmutable(),
                'rsvpCount' => $this->rsvps->countHeld($eventEntity),
                'isEventMaster' => true,
                // AC-02-40: attendance visible to Super Admin too.
                'attendanceRecords' => $this->attendanceRecords->findForEvent($eventEntity),
            ]);
        });
    }

    /**
     * AC-02-57/AC-07-25: edits as if they had created it. AC-07-27/28: a
     * coach-assignment scheduling conflict — a double-booked coach, or an
     * unavailable-at-this-time coach — never shows a warning and never
     * blocks the save; `EventService::update(..., overrideConflicts:
     * true)` is what makes that true, and is what still writes the
     * `event.coach_conflict_override` audit entry when a conflict genuinely
     * existed (see that method's own docblock — nothing is logged when
     * there was no conflict to override).
     */
    #[Route('/super-admin/events/{event<\d+>}/edit', name: 'administration_event_master_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, int $event): Response
    {
        return $this->withScope($event, function (Event $eventEntity) use ($request): Response {
            $this->denyAccessUnlessGranted(EventVoter::EVENT_EDIT, $eventEntity);

            // AC-02-54: structural, universal — see
            // EventMasterController::cancel()'s own docblock for why this
            // is not something "edits as if they had created it" (AC-02-57)
            // overrides.
            if (!$eventEntity->isEditable(new \DateTimeImmutable())) {
                $this->addFlash('error', 'This event can no longer be edited — it is canceled or has already started.');

                return $this->redirectToRoute('administration_event_master_show', ['event' => $eventEntity->getId()]);
            }

            // See TrainerEventController::dataFromEvent()'s own comment:
            // the form's model_timezone is the trainer's own zone, so the
            // pre-filled value must already carry that exact timezone.
            $tz = $eventEntity->getTrainer()->getTimezone();
            $currentAssignment = $this->coachAssignments->findCurrentForEvent($eventEntity);

            $form = $this->createForm(EventType::class, [
                'title' => $eventEntity->getTitle(),
                'eventType' => $eventEntity->getEventType(),
                'startsAt' => $eventEntity->getStartsAt()->setTimezone($tz),
                'endsAt' => $eventEntity->getEndsAt()->setTimezone($tz),
                'location' => $eventEntity->getLocation(),
                'capacity' => $eventEntity->getCapacity(),
                'visibility' => $eventEntity->getVisibility(),
                'description' => $eventEntity->getDescription(),
                'minAge' => $eventEntity->getMinAge(),
                'maxAge' => $eventEntity->getMaxAge(),
                'skillLevels' => null === $eventEntity->getSkillLevels() ? null : implode(', ', $eventEntity->getSkillLevels()),
                'genders' => null === $eventEntity->getGenders() ? null : implode(', ', $eventEntity->getGenders()),
                'usdPricingEnabled' => $eventEntity->isUsdPricingEnabled(),
                'usdPrice' => $eventEntity->getUsdPriceMinorUnits() / 100,
                'tokenPricingEnabled' => $eventEntity->isTokenPricingEnabled(),
                'tokenPrice' => $eventEntity->getTokenPrice(),
                'coach' => $currentAssignment?->getCoachMembership(),
            ], [
                'timezone' => $eventEntity->getTrainer()->getTimezone(),
                'invitablePlayers' => [],
                // AC-07-27: the trainer's own active coaches, reachable now
                // that withScope() has an AdministrativeScope open for this
                // event's trainer — the same ordinary, RLS-bound query
                // TrainerEventController's own assignableCoaches() runs.
                'assignableCoaches' => $this->assignableCoaches(),
                'submitLabel' => 'Save (Super Admin)',
                'skipPastCheck' => true,
            ]);
            $form->handleRequest($request);

            if ($form->isSubmitted() && $form->isValid()) {
                /** @var array<string, mixed> $data */
                $data = $form->getData();

                $skillLevels = $this->parseCommaList($data['skillLevels'] ?? null);
                $genders = $this->parseCommaList($data['genders'] ?? null);

                /** @var CoachMembership|null $coach */
                $coach = $data['coach'] ?? null;

                $input = new EventInput(
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
                    coachMembershipId: $coach?->getId(),
                );

                // AC-07-27: overrideConflicts is unconditional here — Super
                // Admin never sees the "Continue anyway?" round-trip
                // TrainerEventController's own catch(CoachAssignmentConflictException)
                // block shows a trainer.
                $this->eventService->update($eventEntity, $this->actor(), $input, overrideConflicts: true);
                $this->addFlash('success', 'Event updated (Super Admin).');

                return $this->redirectToRoute('administration_event_master_show', ['event' => $eventEntity->getId()]);
            }

            return $this->render('scheduling/trainer_event_form.html.twig', ['form' => $form, 'mode' => 'edit', 'event' => $eventEntity, 'isEventMaster' => true]);
        });
    }

    /**
     * AC-02-48/57: overriding the trainer cancellation-window policy — the
     * epic's own open questions record that specific window as "policy
     * TBD" and nothing implements it anywhere in Scheduling today, so
     * there is nothing concrete for this action to override yet; the
     * override is a no-op until that policy exists. AC-02-49's own
     * "already started or completed" rule is a structural fact, not a
     * policy — it stays universal, Super Admin included, matching
     * EventService::cancel()'s single, un-role-qualified guard.
     */
    #[Route('/super-admin/events/{event<\d+>}/cancel', name: 'administration_event_master_cancel', methods: ['GET', 'POST'])]
    public function cancel(Request $request, int $event): Response
    {
        return $this->withScope($event, function (Event $eventEntity) use ($request): Response {
            $this->denyAccessUnlessGranted(EventVoter::EVENT_CANCEL, $eventEntity);

            if (!$eventEntity->isCancelable(new \DateTimeImmutable())) {
                $this->addFlash('error', 'This event can no longer be canceled — it has already started or completed.');

                return $this->redirectToRoute('administration_event_master_show', ['event' => $eventEntity->getId()]);
            }

            $form = $this->createForm(CancelEventType::class);
            $form->handleRequest($request);

            if ($form->isSubmitted() && $form->isValid()) {
                /** @var array{reason: string} $data */
                $data = $form->getData();
                $this->eventService->cancel($eventEntity, $data['reason'], $this->actor());
                $this->addFlash('success', 'Event canceled (Super Admin).');

                return $this->redirectToRoute('administration_event_master_index');
            }

            return $this->render('scheduling/trainer_event_cancel.html.twig', ['form' => $form, 'event' => $eventEntity, 'isEventMaster' => true]);
        });
    }

    #[Route('/super-admin/events/{event<\d+>}/rsvps', name: 'administration_event_master_rsvps', methods: ['GET'])]
    public function rsvps(int $event): Response
    {
        return $this->withScope($event, function (Event $eventEntity): Response {
            $this->denyAccessUnlessGranted(EventVoter::EVENT_VIEW_RSVP_LIST, $eventEntity);

            $tz = $eventEntity->getTrainer()->getTimezone();
            $rows = array_map(
                static fn (Rsvp $r): array => [
                    'name' => $r->getPlayer()->getFirstName(),
                    'status' => $r->getStatus(),
                    'requestedAt' => $r->getRequestedAt()->setTimezone($tz)->format('Y-m-d H:i'),
                ],
                $this->rsvps->findForEvent($eventEntity),
            );

            return $this->render('administration/event_master_rsvps.html.twig', ['event' => $eventEntity, 'rows' => $rows]);
        });
    }

    /**
     * @template T
     *
     * @param callable(Event): T $callback
     *
     * @return T
     */
    private function withScope(int $eventId, callable $callback): mixed
    {
        // Resolve the trainer via a crossing read first (BYPASSRLS), since
        // no tenant is active yet and the ordinary EventRepository cannot
        // see a foreign-tenant row.
        $trainerId = $this->crossTenantReads->resolveTrainerIdForEvent($eventId);

        if (null === $trainerId) {
            throw $this->createNotFoundException('Event not found.');
        }

        $trainer = $this->trainers->find($trainerId) ?? throw $this->createNotFoundException('Trainer not found.');

        $this->administrativeScope->openFor($trainer, $this->actor());

        try {
            $eventEntity = $this->events->find($eventId) ?? throw $this->createNotFoundException('Event not found.');

            return $callback($eventEntity);
        } finally {
            $this->administrativeScope->close();
        }
    }

    /**
     * AC-07-27: the active tenant's own active coaches — only ever called
     * from inside withScope()'s open AdministrativeScope, so this ordinary,
     * RLS-bound query sees exactly this event's own trainer's coaches.
     * Unlike `TrainerEventController::assignableCoaches()`, no "self as
     * coach" entry is prepended: that convenience is for a trainer editing
     * their own events, not the administrative-override use case here.
     *
     * @return array<int, CoachMembership>
     */
    private function assignableCoaches(): array
    {
        $coaches = [];

        foreach ($this->coachMemberships->findAllForActiveTenant() as $membership) {
            if ($membership->isActive()) {
                $coaches[(int) $membership->getId()] = $membership;
            }
        }

        return $coaches;
    }

    private function stringOrNull(mixed $value): ?string
    {
        return \is_string($value) && '' !== $value ? $value : null;
    }

    /**
     * @return list<string>|null
     */
    private function parseCommaList(mixed $raw): ?array
    {
        if (!\is_string($raw) || '' === trim($raw)) {
            return null;
        }

        $items = array_values(array_filter(array_map('trim', explode(',', $raw)), static fn (string $v): bool => '' !== $v));

        return [] === $items ? null : $items;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
