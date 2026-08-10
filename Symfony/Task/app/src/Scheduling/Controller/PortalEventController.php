<?php

declare(strict_types=1);

namespace App\Scheduling\Controller;

use App\Identity\Entity\Account;
use App\Identity\Service\PlayerContextResolver;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Exception\AlreadyRegisteredException;
use App\Scheduling\Exception\EventFullException;
use App\Scheduling\Form\RsvpType;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\RsvpService;
use App\Scheduling\Voter\EventVoter;
use App\Scheduling\Voter\RsvpVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-02.07: viewing an event's details and RSVPing.
 *
 * @see specs/api-designer-spec.md "Scheduling module" — player/parent portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalEventController extends AbstractController
{
    public function __construct(
        private readonly RsvpService $rsvpService,
        private readonly RsvpRepository $rsvps,
        private readonly PlayerContextResolver $playerContext,
    ) {
    }

    /**
     * BR-02-5/6: EventVoter::EVENT_VIEW denies (structurally 404-shaped, per
     * the Doctrine filter/RLS, or a genuine 403 for a same-tenant
     * non-invited/ineligible attempt) exactly as AC-02-7 states.
     */
    #[Route('/portal/events/{event}', name: 'scheduling_portal_event_show', methods: ['GET'])]
    public function show(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(EventVoter::EVENT_VIEW, $event);

        $player = $this->playerContext->resolve($request, $this->actor());
        $existing = $this->rsvps->findActiveOneByEventAndPlayer($event, $player);
        $isFull = $this->rsvps->countHeld($event) >= $event->getCapacity();

        $form = null === $existing && !$isFull
            ? $this->createForm(RsvpType::class, null, ['event' => $event, 'action' => $this->generateUrl('scheduling_portal_event_rsvp', ['event' => $event->getId()])])
            : null;

        return $this->render('scheduling/portal_event_show.html.twig', [
            'event' => $event,
            'existingRsvp' => $existing,
            'isFull' => $isFull,
            'form' => $form,
            'now' => new \DateTimeImmutable(),
        ]);
    }

    /**
     * AC-02-23..28, BR-02-7..10.
     */
    #[Route('/portal/events/{event}/rsvp', name: 'scheduling_portal_event_rsvp', methods: ['POST'])]
    public function rsvp(Request $request, Event $event): Response
    {
        $this->denyAccessUnlessGranted(RsvpVoter::RSVP_CREATE, $event);

        $actor = $this->actor();
        $player = $this->playerContext->resolve($request, $actor);

        $form = $this->createForm(RsvpType::class, null, ['event' => $event]);
        $form->handleRequest($request);

        if (!$form->isSubmitted() || !$form->isValid()) {
            $this->addFlash('error', 'Please choose a payment method.');

            return $this->redirectToRoute('scheduling_portal_event_show', ['event' => $event->getId()]);
        }

        /** @var array{paymentMethod: string} $data */
        $data = $form->getData();

        try {
            $rsvpEntity = $this->rsvpService->rsvp($event, $player, $actor, $data['paymentMethod']);
        } catch (EventFullException) {
            $this->addFlash('error', 'Event Full - No spots available.');

            return $this->redirectToRoute('scheduling_portal_event_show', ['event' => $event->getId()]);
        } catch (AlreadyRegisteredException) {
            $this->addFlash('error', 'Already registered.');

            return $this->redirectToRoute('scheduling_portal_event_show', ['event' => $event->getId()]);
        }

        match ($rsvpEntity->getStatus()) {
            // BR-02-10: pending parent approval.
            Rsvp::STATUS_PENDING_PARENT_APPROVAL => $this->addFlash('info', 'Your request is pending parent approval.'),
            // AC-02-24/26: confirmed — free events immediately, paid events
            // once payment succeeds.
            Rsvp::STATUS_CONFIRMED => $this->addFlash('success', "You're registered!"),
            // AC-02-25: "redirected to payment" — with the shipped
            // NoopPaymentIntentGateway this is where every paid RSVP
            // honestly stays until Epic-05 exists (see that class's own
            // docblock) — never claiming a confirmation that has not
            // actually happened.
            default => $this->addFlash('info', 'Your registration is awaiting payment confirmation.'),
        };

        if (Rsvp::STATUS_PENDING_PARENT_APPROVAL === $rsvpEntity->getStatus()) {
            return $this->redirectToRoute('identity_portal_approvals_index');
        }

        return $this->redirectToRoute('scheduling_portal_reservations');
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
