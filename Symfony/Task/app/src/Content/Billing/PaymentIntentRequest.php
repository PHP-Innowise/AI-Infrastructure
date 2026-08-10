<?php

declare(strict_types=1);

namespace App\Content\Billing;

use App\Content\Entity\Playlist;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;

/**
 * What Content tells Billing (Epic-05) it needs paid for one playlist
 * purchase. Deliberately narrow — no Stripe types, no token-ledger types —
 * matching `App\Scheduling\Billing\PaymentIntentRequest`'s own shape and
 * reasoning exactly, kept as a separate class per the module boundary (see
 * `PaymentIntentOutcome`'s own docblock).
 *
 * `amount` is a structural placeholder, always `0` from every caller in this
 * codebase today: BR-04-8's four candidate pricing models are explicitly
 * unresolved in the source ("To be finalized with client") and
 * `specs/database-designer-schema.md` accordingly stores no price on
 * `playlist` — there is nothing real to put here yet. The field exists so
 * Epic-05's eventual gateway implementation does not need this request
 * shape to change once real pricing lands.
 */
final readonly class PaymentIntentRequest
{
    public function __construct(
        public Trainer $trainer,
        public Playlist $playlist,
        public PlayerProfile $player,
        public Account $payer,
        public string $paymentMethod,
        public int $amount,
    ) {
    }
}
