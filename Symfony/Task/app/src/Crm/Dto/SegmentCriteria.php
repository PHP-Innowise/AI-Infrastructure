<?php

declare(strict_types=1);

namespace App\Crm\Dto;

/**
 * Every filter US-03.02/US-03.06 name, AND-combined (BR-03-13). All
 * properties are optional/nullable — an unset filter simply does not narrow
 * the result. `PlayerSegmentationRepository::search()`'s sole input.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-8..10, AC-03-22..26, BR-03-13/14
 */
final readonly class SegmentCriteria
{
    /**
     * @param list<int>|null $labelIds Player has ANY of these labels (BR-03-13: AND across categories, OR within one — see the coder's final report)
     * @param list<string>|null $flagTypes Player has an ACTIVE flag of ANY of these types
     */
    public function __construct(
        public ?string $query = null,
        public ?string $skillLevel = null,
        public ?int $minAge = null,
        public ?int $maxAge = null,
        public ?string $gender = null,
        public ?array $labelIds = null,
        public ?array $flagTypes = null,
        public ?string $teamSchoolClub = null,
        public ?int $attendedMoreThan = null,
        public ?int $attendedInLastDays = null,
        public ?float $attendanceRateGreaterThan = null,
        public ?int $noShowsGreaterThan = null,
        public ?\DateTimeImmutable $registeredFrom = null,
        public ?\DateTimeImmutable $registeredTo = null,
        public ?string $lastActivityBand = null,
        public string $sort = self::SORT_NAME_ASC,
        public int $page = 1,
        public int $perPage = 50,
    ) {
    }

    public const SORT_NAME_ASC = 'name_asc';
    public const SORT_NAME_DESC = 'name_desc';
    public const SORT_LAST_ACTIVITY = 'last_activity';
    public const SORT_ATTENDANCE_RATE = 'attendance_rate';

    public const BAND_ACTIVE = 'active';
    public const BAND_INACTIVE = 'inactive';
    public const BAND_CHURNED = 'churned';

    public function hasAnyFilter(): bool
    {
        return null !== $this->query
            || null !== $this->skillLevel
            || null !== $this->minAge
            || null !== $this->maxAge
            || null !== $this->gender
            || null !== $this->labelIds
            || null !== $this->flagTypes
            || null !== $this->teamSchoolClub
            || null !== $this->attendedMoreThan
            || null !== $this->attendedInLastDays
            || null !== $this->attendanceRateGreaterThan
            || null !== $this->noShowsGreaterThan
            || null !== $this->registeredFrom
            || null !== $this->registeredTo
            || null !== $this->lastActivityBand;
    }
}
