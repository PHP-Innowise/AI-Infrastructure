<?php

declare(strict_types=1);

namespace App\Crm\Dto;

/**
 * One player row for the CRM list/segmentation screen — a DTO projection,
 * never a hydrated entity graph (specs/architect-architecture.md
 * "Persistence and queries: read-heavy screens return DTO projections").
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-3
 */
final readonly class PlayerSummary
{
    public function __construct(
        public int $membershipId,
        public int $playerId,
        public string $name,
        public ?string $photoUrl,
        public int $age,
        public ?string $gender,
        public ?string $skillLevel,
        public ?string $teamSchoolClub,
        public int $attendedCount,
        public int $totalTrackedCount,
        public int $noShowCount,
        public float $attendanceRate,
        public \DateTimeImmutable $registeredAt,
        public \DateTimeImmutable $lastActivityAt,
    ) {
    }
}
