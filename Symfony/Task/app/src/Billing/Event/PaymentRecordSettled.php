<?php

declare(strict_types=1);

namespace App\Billing\Event;

/**
 * Dispatched by Billing whenever a `PaymentRecord`'s outcome becomes known
 * ASYNCHRONOUSLY — i.e. from the Stripe webhook handler, never from a
 * synchronous request/response cycle (a synchronous token spend reports its
 * outcome via a plain return value instead, per
 * `App\Scheduling\Billing\PaymentIntentResult`/
 * `App\Content\Billing\PaymentIntentResult`).
 *
 * This is the "domain event dispatched from Billing and subscribed to by
 * the caller" half of architect-architecture.md's stated contract
 * ("Payment outcomes reach the caller by return value inside the request,
 * or by a domain event dispatched from Billing and subscribed to by the
 * caller — never by Billing reaching into another module"). A plain
 * Symfony `EventDispatcherInterface` event, not a Messenger message: it is
 * dispatched FROM WITHIN the (already-async) webhook Messenger handler, in
 * the worker process, so a second layer of async dispatch would add
 * nothing.
 *
 * `App\Scheduling\EventSubscriber\PaymentOutcomeSubscriber`,
 * `App\Content\EventSubscriber\PaymentOutcomeSubscriber` and
 * `App\Forms\EventSubscriber\PaymentOutcomeSubscriber` are the three
 * subscribers today, filtering on `$type` for the rows they own
 * (`event_rsvp`, `content_purchase`, `camp_registration` respectively) and
 * ignoring every other `$type`.
 */
final class PaymentRecordSettled
{
    public const OUTCOME_SUCCEEDED = 'succeeded';
    public const OUTCOME_FAILED = 'failed';
    public const OUTCOME_REFUNDED = 'refunded';

    /**
     * @param array<string, string> $metadata The Stripe Checkout metadata
     *                                        the ORIGINATING request attached (see
     *                                        `App\Billing\Stripe\StripeCheckoutSessionRequest`'s own docblock) —
     *                                        echoed through verbatim so a subscriber can recover request-time
     *                                        information `PaymentRecord` itself has no column for, without
     *                                        Billing's own event needing to know what any of it means. Content's
     *                                        subscriber reads `metadata['player_id']` (a card-funded content
     *                                        purchase's beneficiary player has no other column to live in —
     *                                        `payment_record` links a playlist and a payer account, never a
     *                                        player profile, unlike `token_entry.beneficiary_player_id`).
     */
    public function __construct(
        public readonly int $paymentRecordId,
        public readonly string $type,
        public readonly string $outcome,
        public readonly ?int $relatedRsvpId,
        public readonly ?int $relatedPlaylistId,
        public readonly array $metadata = [],
        public readonly ?int $relatedFormSubmissionId = null,
    ) {
    }
}
