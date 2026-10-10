<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Entity\StripeEventReceipt;
use App\Billing\Message\ProcessStripeWebhookEvent;
use App\Billing\Repository\StripeEventReceiptRepository;
use App\Billing\Stripe\StripeClient;
use App\Billing\Stripe\StripeSignatureVerificationException;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Messenger\MessageBusInterface;
use Symfony\Component\Routing\Attribute\Route;

/**
 * The one Stripe webhook receiver — every event this platform consumes,
 * including Epic-08's camp payment confirmations (BR-08-14), since Forms
 * (not built in this codebase) would reuse the same Stripe Checkout
 * mechanism as everything else (BR-08-11/BR-05-6).
 *
 * Sits behind the `webhook` firewall segment (`config/packages/security.yaml`):
 * `stateless: true`, `security: false` — the signature check inside this
 * controller IS the authorization, per
 * specs/api-designer-spec.md "Stripe webhook contract".
 *
 * @see specs/api-designer-spec.md "Stripe webhook contract"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-34..37
 */
final class WebhookController extends AbstractController
{
    public function __construct(
        private readonly StripeClient $stripeClient,
        private readonly StripeEventReceiptRepository $receipts,
        private readonly EntityManagerInterface $entityManager,
        private readonly MessageBusInterface $messageBus,
        private readonly string $webhookSecret,
    ) {
    }

    #[Route('/webhooks/stripe', name: 'billing_webhook_stripe_receive', methods: ['POST'])]
    public function __invoke(Request $request): Response
    {
        // Step 1: the raw body, never $request->toArray() or anything that
        // re-serializes it — the signature is computed over the exact bytes
        // received.
        $payload = $request->getContent();
        $sigHeader = $request->headers->get('Stripe-Signature', '');

        // Step 2: verify.
        try {
            $event = $this->stripeClient->verifyWebhookSignature($payload, $sigHeader, $this->webhookSecret);
        } catch (StripeSignatureVerificationException) {
            return new Response('', Response::HTTP_BAD_REQUEST);
        }

        // Step 3: insert the idempotency row FIRST, in its own transaction,
        // before the event's business meaning is parsed.
        $receipt = new StripeEventReceipt($event->id, $event->type, $payload);

        try {
            $this->receipts->add($receipt);
            $this->entityManager->flush();
        } catch (UniqueConstraintViolationException) {
            // Step 4: a redelivery of an event already received — 200
            // immediately, nothing re-dispatched. Expected at-least-once
            // delivery shape, not an error.
            return new Response('', Response::HTTP_OK);
        }

        // Step 5: dispatch, carrying only the receipt's own id.
        $this->messageBus->dispatch(new ProcessStripeWebhookEvent((int) $receipt->getId()));

        return new Response('', Response::HTTP_OK);
    }
}
