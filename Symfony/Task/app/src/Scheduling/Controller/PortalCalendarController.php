<?php

declare(strict_types=1);

namespace App\Scheduling\Controller;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\AvailabilityWindowRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\PlayerContextResolver;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventInvitationRepository;
use App\Scheduling\Repository\EventRepository;
use App\Scheduling\Service\EventEligibilityChecker;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.06: the Training Calendar — public+eligible events, plus private
 * events the current player context is invited to.
 *
 * @see specs/api-designer-spec.md "Scheduling module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalCalendarController extends AbstractController
{
    public function __construct(
        private readonly EventRepository $events,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly EventInvitationRepository $invitations,
        private readonly EventEligibilityChecker $eligibility,
        private readonly AvailabilityWindowRepository $availabilityWindows,
        private readonly PlayerContextResolver $playerContext,
    ) {
    }

    /**
     * AC-02-18..22, AC-02-64: month/week/day toggle, search, filters, an
     * availability-match badge when the player has set preferences. AC-02-64
     * ("fully separated Training Calendar views per trainer context") is
     * automatic here — this reads only the active tenant's own events, the
     * same isolation every trainer-scoped read gets; switching trainers is
     * platform_context_trainer_switch's own job (Epic-01), not repeated here.
     */
    #[Route('/portal/calendar', name: 'scheduling_portal_calendar', methods: ['GET'])]
    public function __invoke(Request $request): Response
    {
        $actor = $this->actor();
        $player = $this->playerContext->resolve($request, $actor);
        $membership = $this->currentMembership($player);

        $view = (string) $request->query->get('view', 'month');
        $query = $request->query->get('q');
        $type = $request->query->get('type');
        $location = $request->query->get('location');

        $now = new \DateTimeImmutable();
        [$rangeStart, $rangeEnd] = $this->rangeFor($view, $now);

        $invitedEventIds = $this->invitations->findInvitedEventIdsForPlayer($player);
        $hasAvailability = [] !== $this->availabilityWindows->findForPlayer($player);

        $rows = [];
        foreach ($this->events->findUpcomingForActiveTenant($now) as $event) {
            if ($event->getStartsAt() < $rangeStart || $event->getStartsAt() > $rangeEnd) {
                continue;
            }

            if (!$this->isVisible($event, $membership, $invitedEventIds)) {
                continue;
            }

            if (\is_string($query) && '' !== $query && !$this->matchesSearch($event, $query)) {
                continue;
            }

            if (\is_string($type) && '' !== $type && $event->getEventType() !== $type) {
                continue;
            }

            if (\is_string($location) && '' !== $location && !str_contains(mb_strtolower($event->getLocation()), mb_strtolower($location))) {
                continue;
            }

            $rows[] = [
                'event' => $event,
                'availabilityBadge' => $hasAvailability ? $this->matchesAvailability($event, $player) : null,
            ];
        }

        return $this->render('scheduling/portal_calendar.html.twig', [
            'view' => $view,
            'rows' => $rows,
        ]);
    }

    private function currentMembership(PlayerProfile $player): ?PlayerTrainerMembership
    {
        // Same-tenant lookup; the active tenant is whichever trainer context
        // the player is currently in (Epic-01's own switcher).
        foreach ($this->memberships->findActiveForPlayer($player) as $membership) {
            return $membership;
        }

        return null;
    }

    /**
     * @param list<int> $invitedEventIds
     */
    private function isVisible(Event $event, ?PlayerTrainerMembership $membership, array $invitedEventIds): bool
    {
        if ($event->isPrivate()) {
            return \in_array((int) $event->getId(), $invitedEventIds, true);
        }

        return null !== $membership && $membership->isActive() && $this->eligibility->isEligible($event, $membership);
    }

    private function matchesSearch(Event $event, string $query): bool
    {
        $needle = mb_strtolower($query);

        return str_contains(mb_strtolower($event->getTitle()), $needle)
            || str_contains(mb_strtolower($event->getLocation()), $needle);
    }

    /**
     * @return array{0: \DateTimeImmutable, 1: \DateTimeImmutable}
     */
    private function rangeFor(string $view, \DateTimeImmutable $now): array
    {
        return match ($view) {
            'day' => [$now, $now->modify('+1 day')],
            'week' => [$now, $now->modify('+7 days')],
            default => [$now, $now->modify('+1 month')],
        };
    }

    private function matchesAvailability(Event $event, PlayerProfile $player): string
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
        $start = new \DateTimeImmutable('1970-01-01 '.$normalizedStart->format('H:i:s'));
        $end = new \DateTimeImmutable('1970-01-01 '.$normalizedEnd->format('H:i:s'));

        foreach ($this->availabilityWindows->findForPlayer($player) as $window) {
            if ($window->isAvailable() && $window->getDayOfWeek() === $dayOfWeek && $window->getStartTime() <= $start && $window->getEndTime() >= $end) {
                return 'Matches your availability';
            }
        }

        return 'Conflicts with your availability';
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
