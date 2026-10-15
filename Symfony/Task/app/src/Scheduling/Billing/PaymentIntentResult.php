<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

/**
 * Wraps `PaymentIntentOutcome` with the two pieces of information Epic-05
 * adds that a bare enum cannot carry (PHP enum cases are singletons — see
 * this class's own docblock precedent): which `payment_record` was created
 * (so `RsvpService` can call `Rsvp::attachPaymentRecordId()`, an existing
 * method that had no caller until now), and, for a card payment, the Stripe
 * Checkout URL to redirect to — needed because
 * specs/api-designer-spec.md's "Billing module" states plainly that "every
 * purchase-initiating route in Scheduling/Content/Billing itself... performs
 * the 303 redirect to the URL Stripe returns in the same request; there is
 * no separate 'create checkout session' endpoint." `redirectUrl` is never
 * persisted anywhere (`PaymentRecord` itself has no such column — see that
 * entity's own docblock) — it exists in PHP memory for exactly the one
 * request that needs it.
 */
final readonly class PaymentIntentResult
{
    private function __construct(
        public PaymentIntentOutcome $outcome,
        public ?int $paymentRecordId,
        public ?string $redirectUrl,
    ) {
    }

    /**
     * $paymentRecordId is nullable so a test stub can report Succeeded
     * without first constructing a real, persisted `PaymentRecord` (which
     * would otherwise be required — `rsvp.payment_record_id` carries a real
     * foreign key once Epic-05's migration attaches it, so an arbitrary,
     * non-existent id would fail at flush time). The real, Billing-provided
     * implementation always supplies one — a `Succeeded` outcome only ever
     * happens after a real payment record exists.
     */
    public static function succeeded(?int $paymentRecordId = null): self
    {
        return new self(PaymentIntentOutcome::Succeeded, $paymentRecordId, null);
    }

    public static function pending(int $paymentRecordId, ?string $redirectUrl = null): self
    {
        return new self(PaymentIntentOutcome::Pending, $paymentRecordId, $redirectUrl);
    }

    public static function failed(): self
    {
        return new self(PaymentIntentOutcome::Failed, null, null);
    }
}
