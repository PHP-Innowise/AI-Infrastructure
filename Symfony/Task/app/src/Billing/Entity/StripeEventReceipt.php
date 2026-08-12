<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\StripeEventReceiptRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * Inbound idempotency for the Stripe webhook receiver: "the unique
 * violation *is* the duplicate detection" (architect-architecture.md
 * "Payment records"). Global — no trainer is known yet when this row is
 * inserted (the platform learns which trainer an event belongs to only
 * after loading the `PaymentRecord` the event references).
 *
 * Inserted in its OWN transaction, before the event's business meaning is
 * parsed (specs/api-designer-spec.md "Stripe webhook contract" step 3) —
 * `BillingWebhookController` does exactly this, catching the unique
 * violation and returning `200` immediately for a replay (step 4).
 *
 * `rawPayload` is a deliberate addition beyond
 * specs/database-designer-schema.md's own six-column listing for this
 * table — see Version20260811100000's own docblock for why it is required,
 * not optional. No `status` column: `processedAt IS NULL` (pending) and
 * `processingError IS NOT NULL` once processed (failed) already represent
 * every state the api-designer-spec's prose names.
 *
 * @see specs/database-designer-schema.md "`stripe_event_receipt` — Billing"
 * @see specs/api-designer-spec.md "Stripe webhook contract"
 */
#[ORM\Entity(repositoryClass: StripeEventReceiptRepository::class)]
#[ORM\Table(name: 'stripe_event_receipt')]
#[ORM\UniqueConstraint(name: 'uniq_stripe_event_receipt_event_id', columns: ['stripe_event_id'])]
class StripeEventReceipt
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\Column(name: 'stripe_event_id', type: 'string', length: 255)]
    private string $stripeEventId;

    #[ORM\Column(name: 'event_type', type: 'string', length: 100)]
    private string $eventType;

    #[ORM\Column(name: 'raw_payload', type: 'text')]
    private string $rawPayload;

    #[ORM\Column(name: 'received_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $receivedAt;

    #[ORM\Column(name: 'processed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $processedAt = null;

    #[ORM\Column(name: 'processing_error', type: 'text', nullable: true)]
    private ?string $processingError = null;

    public function __construct(string $stripeEventId, string $eventType, string $rawPayload, ?\DateTimeImmutable $receivedAt = null)
    {
        $this->stripeEventId = $stripeEventId;
        $this->eventType = $eventType;
        $this->rawPayload = $rawPayload;
        $this->receivedAt = $receivedAt ?? new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getStripeEventId(): string
    {
        return $this->stripeEventId;
    }

    public function getEventType(): string
    {
        return $this->eventType;
    }

    public function getRawPayload(): string
    {
        return $this->rawPayload;
    }

    public function getReceivedAt(): \DateTimeImmutable
    {
        return $this->receivedAt;
    }

    public function getProcessedAt(): ?\DateTimeImmutable
    {
        return $this->processedAt;
    }

    public function getProcessingError(): ?string
    {
        return $this->processingError;
    }

    public function isPending(): bool
    {
        return null === $this->processedAt;
    }

    public function markProcessed(): void
    {
        $this->processedAt = new \DateTimeImmutable();
        $this->processingError = null;
    }

    /**
     * AC-05-35: retried with exponential backoff. Deliberately does NOT set
     * `processedAt` — the row must stay found by the `processed_at IS NULL`
     * partial index (specs/database-designer-schema.md "`stripe_event_receipt`":
     * "retry sweep for AC-05-35's exponential backoff") until it eventually
     * succeeds. The actual retry attempts are Messenger's own
     * `retry_strategy` (config/packages/messenger.yaml: 3 attempts,
     * multiplier 2) re-invoking the handler on the SAME message; this
     * column only records the most recent failure reason for operator
     * visibility (AC-05-37: "Stripe API errors are logged and the admin is
     * notified").
     */
    public function markFailed(string $error): void
    {
        $this->processingError = $error;
    }
}
