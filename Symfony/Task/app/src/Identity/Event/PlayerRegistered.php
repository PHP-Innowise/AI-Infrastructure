<?php

declare(strict_types=1);

namespace App\Identity\Event;

/**
 * Dispatched once a brand-new account + player profile has been created and
 * committed via `PlayerRegistrationService::registerViaShareLink()` — the
 * "domain event dispatched from X and subscribed to by the caller" shape
 * `App\Billing\Event\PaymentRecordSettled` already establishes, applied here
 * so Growth (Epic-06) can complete referral attribution (AC-06-5/6/7)
 * without Identity ever calling into Growth directly: the module map allows
 * `Identity` to call only `Platform`, so this is a plain
 * `EventDispatcherInterface` event Identity fires and forgets, not a
 * targeted call to any specific listener.
 *
 * Dispatched AFTER `registerViaShareLink()`'s own transaction has already
 * committed — a listener here runs against a real, persisted account, and a
 * listener failure can never roll back a registration that has already
 * succeeded from the registrant's point of view.
 *
 * Carries only ids, matching `PaymentRecordSettled`'s own "the payload is
 * stable" precedent — a listener re-reads whatever entities it needs rather
 * than trusting values that traveled alongside the event.
 */
final class PlayerRegistered
{
    public function __construct(
        public readonly int $accountId,
        public readonly int $playerProfileId,
        public readonly int $trainerId,
        public readonly \DateTimeImmutable $registeredAt,
    ) {
    }
}
