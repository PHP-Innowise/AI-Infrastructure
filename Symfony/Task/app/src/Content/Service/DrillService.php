<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\Drill;
use App\Content\Repository\DrillRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-04.02/03/06/10: the Drill Database — create, edit, publish-toggle, and
 * delete drills. Video `ContentItem`s have no standalone lifecycle of their
 * own in this epic (see `PlaylistService::addVideoItem()`'s own docblock) —
 * this service is deliberately Drill-specific, not a generic ContentItem
 * CRUD surface.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md AC-04-4..8/17..19/35
 */
final readonly class DrillService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private DrillRepository $drills,
    ) {
    }

    /**
     * AC-04-4..6: required name and YouTube URL, at least one category —
     * all enforced by `Drill`'s own constructor guards.
     *
     * @param list<string>|null $tags
     * @param list<string>|null $equipment
     * @param list<string>      $categories
     */
    public function createDrill(
        Trainer $trainer,
        string $title,
        string $youtubeUrl,
        string $difficultyLevel,
        array $categories,
        ?string $instructions,
        ?array $tags,
        ?array $equipment,
        ?string $spaceRequirement,
        ?string $playerCount,
        ?int $durationMinMinutes,
        ?int $durationMaxMinutes,
        bool $isPublic,
    ): Drill {
        return $this->entityManager->wrapInTransaction(function () use (
            $trainer, $title, $youtubeUrl, $difficultyLevel, $categories, $instructions, $tags,
            $equipment, $spaceRequirement, $playerCount, $durationMinMinutes, $durationMaxMinutes, $isPublic,
        ): Drill {
            $drill = new Drill(
                $trainer,
                $title,
                $youtubeUrl,
                $difficultyLevel,
                $categories,
                $instructions,
                $tags,
                $equipment,
                $spaceRequirement,
                $playerCount,
                $durationMinMinutes,
                $durationMaxMinutes,
            );

            if ($isPublic) {
                // AC-04-5: saving as Public makes it immediately discoverable.
                $drill->setPublic(true, new \DateTimeImmutable());
            }

            $this->drills->add($drill);
            $this->entityManager->flush();

            return $drill;
        });
    }

    /**
     * @param list<string>|null $tags
     * @param list<string>|null $equipment
     * @param list<string>      $categories
     */
    public function updateDrill(
        Drill $drill,
        string $title,
        string $youtubeUrl,
        ?string $instructions,
        ?array $tags,
        string $difficultyLevel,
        array $categories,
        ?array $equipment,
        ?string $spaceRequirement,
        ?string $playerCount,
        ?int $durationMinMinutes,
        ?int $durationMaxMinutes,
    ): void {
        $this->entityManager->wrapInTransaction(function () use (
            $drill, $title, $youtubeUrl, $instructions, $tags, $difficultyLevel, $categories,
            $equipment, $spaceRequirement, $playerCount, $durationMinMinutes, $durationMaxMinutes,
        ): void {
            $drill->updateCommonFields($title, $youtubeUrl, $instructions, $drill->getDurationSeconds(), $tags);
            $drill->updateDrillFields($difficultyLevel, $categories, $equipment, $spaceRequirement, $playerCount, $durationMinMinutes, $durationMaxMinutes);
            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-17/18, BR-04-3..5: same one-way publication mechanism as
     * `Playlist::setVisibility()`.
     */
    public function setPublic(Drill $drill, bool $isPublic): void
    {
        $this->entityManager->wrapInTransaction(function () use ($drill, $isPublic): void {
            $drill->setPublic($isPublic, new \DateTimeImmutable());
            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-35, AC-04-36 (soft-delete recommendation): the drill row itself
     * is marked deleted, never removed — every `PlaylistItem` still pointing
     * at it (this trainer's own, or another trainer's reused reference)
     * keeps its foreign key intact and renders "Drill unavailable" at the
     * read layer, matching BR-04-12/"Drill Reuse".
     */
    public function delete(Drill $drill): void
    {
        $this->entityManager->wrapInTransaction(function () use ($drill): void {
            $drill->softDelete(new \DateTimeImmutable());
            $this->entityManager->flush();
        });
    }
}
