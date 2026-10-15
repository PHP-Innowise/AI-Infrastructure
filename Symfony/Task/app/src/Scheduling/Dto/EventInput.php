<?php

declare(strict_types=1);

namespace App\Scheduling\Dto;

/**
 * The Event Builder form's data, shared by create, edit and duplicate
 * (architect-architecture.md "Input validation": "Request DTOs plus
 * Symfony Forms... custom constraints for cross-field rules"). Every field
 * EventType collects.
 */
final readonly class EventInput
{
    /**
     * @param list<string>|null $skillLevels
     * @param list<string>|null $genders
     * @param list<int>         $invitedPlayerIds
     */
    public function __construct(
        public string $title,
        public string $eventType,
        public \DateTimeImmutable $startsAt,
        public \DateTimeImmutable $endsAt,
        public string $location,
        public int $capacity,
        public string $visibility,
        public ?string $description,
        public ?int $minAge,
        public ?int $maxAge,
        public ?array $skillLevels,
        public ?array $genders,
        public bool $usdPricingEnabled,
        public int $usdPriceMinorUnits,
        public bool $tokenPricingEnabled,
        public int $tokenPrice,
        public array $invitedPlayerIds = [],
        public ?int $coachMembershipId = null,
        public ?string $coachOverrideReason = null,
    ) {
    }

    /**
     * AC-02-61: EventService::createRecurring() clones the first
     * occurrence's input once per generated event, changing only the two
     * date/time fields — every other field (pricing, eligibility, coach,
     * invitees) is identical across the whole series by design.
     */
    public function withDates(\DateTimeImmutable $startsAt, \DateTimeImmutable $endsAt): self
    {
        return new self(
            title: $this->title,
            eventType: $this->eventType,
            startsAt: $startsAt,
            endsAt: $endsAt,
            location: $this->location,
            capacity: $this->capacity,
            visibility: $this->visibility,
            description: $this->description,
            minAge: $this->minAge,
            maxAge: $this->maxAge,
            skillLevels: $this->skillLevels,
            genders: $this->genders,
            usdPricingEnabled: $this->usdPricingEnabled,
            usdPriceMinorUnits: $this->usdPriceMinorUnits,
            tokenPricingEnabled: $this->tokenPricingEnabled,
            tokenPrice: $this->tokenPrice,
            invitedPlayerIds: $this->invitedPlayerIds,
            coachMembershipId: $this->coachMembershipId,
            coachOverrideReason: $this->coachOverrideReason,
        );
    }
}
