<?php

declare(strict_types=1);

namespace App\Scheduling\Dto;

use App\Scheduling\Entity\Event;

/**
 * A frozen copy of the fields AC-02-51's edit-notification diff cares
 * about, taken BEFORE `Event::update()` mutates the managed entity in
 * place — see EventService::update()'s own comment for why this exists:
 * `EventRepository::lockForUpdate()` returns the same instance the caller
 * already held (Doctrine's identity map), so reading "the old value" off
 * the entity itself after the transaction has already applied the new one
 * would silently compare the new value against itself.
 */
final readonly class EventSnapshot
{
    public function __construct(
        public \DateTimeImmutable $startsAt,
        public \DateTimeImmutable $endsAt,
        public string $location,
        public bool $usdPricingEnabled,
        public int $usdPriceMinorUnits,
        public bool $tokenPricingEnabled,
        public int $tokenPrice,
    ) {
    }

    public static function from(Event $event): self
    {
        return new self(
            $event->getStartsAt(),
            $event->getEndsAt(),
            $event->getLocation(),
            $event->isUsdPricingEnabled(),
            $event->getUsdPriceMinorUnits(),
            $event->isTokenPricingEnabled(),
            $event->getTokenPrice(),
        );
    }
}
