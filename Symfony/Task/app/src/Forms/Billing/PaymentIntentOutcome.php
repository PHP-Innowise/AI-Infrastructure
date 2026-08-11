<?php

declare(strict_types=1);

namespace App\Forms\Billing;

/**
 * Kept as Forms' own copy rather than reusing Scheduling's or Content's,
 * matching the precedent `App\Content\Billing\PaymentIntentOutcome` already
 * sets over `App\Scheduling\Billing\PaymentIntentOutcome` — a shared enum
 * would create a dependency between two modules that otherwise only talk to
 * Billing, never to each other (module map: Forms "May call: Platform,
 * Identity, Billing").
 *
 * Camp registration has no `token` payment method (BR-08-11: Stripe Checkout
 * only), so `Succeeded` is never reached synchronously the way a token spend
 * reaches it elsewhere — a free ($0) registration confirms without ever
 * calling this gateway at all (`FormSubmissionService` branches before
 * reaching Billing), so in practice this class only ever returns `Pending`
 * (Checkout session created) or `Failed`.
 */
enum PaymentIntentOutcome
{
    case Succeeded;
    case Pending;
    case Failed;
}
