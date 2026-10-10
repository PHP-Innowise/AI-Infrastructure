<?php

declare(strict_types=1);

namespace App\Growth\EventSubscriber;

use App\Growth\Service\ReferralAttributionService;
use App\Identity\Event\PlayerRegistered;
use App\Identity\Repository\PlayerProfileRepository;
use App\Platform\Repository\TrainerRepository;
use Symfony\Component\EventDispatcher\Attribute\AsEventListener;
use Symfony\Component\HttpFoundation\RequestStack;

/**
 * AC-06-5/6/7: Growth's half of the "domain event dispatched from Identity
 * and subscribed to by the caller" contract — see `PlayerRegistered`'s own
 * docblock. Reads the attribution cookie off the SAME request that just
 * completed registration (this listener runs synchronously, in-request,
 * immediately after `PlayerRegistrationService::registerViaShareLink()`
 * commits) via `RequestStack`, not from any data carried on the event
 * itself.
 *
 * Every failure mode `ReferralAttributionService::completeAttribution()`
 * anticipates (no cookie, expired window, self-referral, wrong trainer,
 * already-referred) is a silent no-op there already — nothing here needs a
 * defensive try/catch on top of that. A genuine, unanticipated failure is
 * allowed to surface rather than being swallowed, since the registration
 * itself has already committed by the time this runs (see
 * `PlayerRegistered`'s own docblock) and hiding a real defect would only
 * make it harder to find.
 */
final readonly class ReferralRegistrationSubscriber
{
    public function __construct(
        private RequestStack $requestStack,
        private ReferralAttributionService $attribution,
        private PlayerProfileRepository $playerProfiles,
        private TrainerRepository $trainers,
    ) {
    }

    #[AsEventListener]
    public function onPlayerRegistered(PlayerRegistered $event): void
    {
        // No HTTP request to read a cookie from (e.g. a console-driven
        // registration, if one is ever added) — nothing to attribute.
        $request = $this->requestStack->getMainRequest();

        if (null === $request) {
            return;
        }

        $trainer = $this->trainers->find($event->trainerId);
        $player = $this->playerProfiles->find($event->playerProfileId);

        if (null === $trainer || null === $player) {
            return;
        }

        $this->attribution->completeAttribution($request, $trainer, $player);
    }
}
