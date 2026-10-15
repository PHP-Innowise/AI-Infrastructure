<?php

declare(strict_types=1);

namespace App\Scheduling\Billing;

use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Scheduling\Entity\Rsvp;

/**
 * What Scheduling tells Billing (Epic-05) it needs paid or refunded for one
 * RSVP. Deliberately narrow — no Stripe types, no token-ledger types appear
 * anywhere in Scheduling, per the architecture's module boundary ("Must not:
 * write token entries directly").
 */
final readonly class PaymentIntentRequest
{
    /**
     * @param ?string $couponCode Epic-06: set only when a valid coupon was
     *                            applied and `$amount` already reflects its
     *                            discount — see
     *                            `App\Content\Billing\PaymentIntentRequest::$couponCode`'s
     *                            own docblock for the full reasoning
     *                            (identical shape here).
     */
    public function __construct(
        public Trainer $trainer,
        public Rsvp $rsvp,
        public Account $payer,
        public string $paymentMethod,
        public int $amount,
        public ?string $couponCode = null,
    ) {
    }
}
