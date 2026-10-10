<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\ContentItem;
use App\Content\Entity\ContentProgress;
use App\Content\Entity\Playlist;
use App\Content\Repository\ContentProgressRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-04.07 (Auto-completion)/US-04.08: BR-04-16's automatic completion on
 * playback start, and the Progress dashboard's derived stats.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-16/17, AC-04-25/27..30
 */
final readonly class ContentProgressService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ContentProgressRepository $progressRepository,
        private PlaylistItemRepository $playlistItems,
        private ContentAssignmentResolver $assignmentResolver,
    ) {
    }

    /**
     * AC-04-25, BR-04-16: "When player clicks play -> Content automatically
     * marked complete." Idempotent regardless of how many times it is
     * called for the same (player, item) pair — Testing Considerations'
     * own "Player clicks play on the same video/drill more than once ->
     * Completion stays idempotent."
     */
    public function recordEngagementStarted(Trainer $trainer, PlayerProfile $player, ContentItem $contentItem): ContentProgress
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $player, $contentItem): ContentProgress {
            $progress = $this->progressRepository->findOneByPlayerAndContentItem($player, $contentItem);

            if (null === $progress) {
                $progress = new ContentProgress($trainer, $player, $contentItem);
                $this->progressRepository->add($progress);
            }

            $progress->recordEngagementStarted(new \DateTimeImmutable());
            $this->entityManager->flush();

            return $progress;
        });
    }

    /**
     * AC-04-29: "3 of 5 items completed" for one specific playlist.
     *
     * @return array{completed: int, total: int}
     */
    public function playlistProgress(Trainer $trainer, PlayerProfile $player, Playlist $playlist): array
    {
        $items = $this->playlistItems->findForPlaylist($playlist);
        $completedItemIds = $this->completedContentItemIds($trainer, $player);

        $completed = 0;
        foreach ($items as $item) {
            if (isset($completedItemIds[(int) $item->getContentItem()->getId()])) {
                ++$completed;
            }
        }

        return ['completed' => $completed, 'total' => \count($items)];
    }

    /**
     * AC-04-27: overall stats — total playlists assigned (across both
     * pillars), total items completed, total watch time, completion rate.
     *
     * @param list<Playlist> $assignedPlaylists
     *
     * @return array{playlistsAssigned: int, itemsCompleted: int, watchTimeSeconds: int, completionRate: float}
     */
    public function overallStats(Trainer $trainer, PlayerProfile $player, array $assignedPlaylists): array
    {
        $rows = $this->progressRepository->findForPlayer($trainer, $player);

        $itemsCompleted = 0;
        $watchTimeSeconds = 0;
        foreach ($rows as $row) {
            if ($row->isCompleted()) {
                ++$itemsCompleted;
            }
            $watchTimeSeconds += $row->getWatchTimeSeconds();
        }

        $totalAssignedItems = 0;
        foreach ($assignedPlaylists as $playlist) {
            $totalAssignedItems += \count($this->playlistItems->findForPlaylist($playlist));
        }

        $completionRate = $totalAssignedItems > 0 ? $itemsCompleted / $totalAssignedItems : 0.0;

        return [
            'playlistsAssigned' => \count($assignedPlaylists),
            'itemsCompleted' => $itemsCompleted,
            'watchTimeSeconds' => $watchTimeSeconds,
            'completionRate' => min(1.0, $completionRate),
        ];
    }

    /**
     * AC-04-28: recently completed content, most recent first.
     *
     * @return list<ContentProgress>
     */
    public function recentActivity(Trainer $trainer, PlayerProfile $player, int $limit = 10): array
    {
        $rows = array_values(array_filter(
            $this->progressRepository->findForPlayer($trainer, $player),
            static fn (ContentProgress $p): bool => $p->isCompleted(),
        ));

        usort($rows, static fn (ContentProgress $a, ContentProgress $b): int => ($b->getCompletedAt() ?? $b->getUpdatedAt()) <=> ($a->getCompletedAt() ?? $a->getUpdatedAt()));

        return \array_slice($rows, 0, $limit);
    }

    /**
     * AC-04-28: assigned playlists NOT yet fully completed, sorted by
     * NEAREST due date first — a playlist with no due date on any of its
     * matching assignment rows sorts last, matching AC-08-18-style "no due
     * date" conventions elsewhere in this codebase (undated items last, not
     * first).
     *
     * @param list<Playlist> $assignedPlaylists
     *
     * @return list<Playlist>
     */
    public function incompleteAssignedPlaylists(Trainer $trainer, PlayerProfile $player, array $assignedPlaylists): array
    {
        $incomplete = array_values(array_filter(
            $assignedPlaylists,
            function (Playlist $playlist) use ($trainer, $player): bool {
                $progress = $this->playlistProgress($trainer, $player, $playlist);

                return $progress['total'] > $progress['completed'];
            },
        ));

        $dueDates = [];
        foreach ($incomplete as $playlist) {
            $dueDates[(int) $playlist->getId()] = $this->assignmentResolver->nearestDueDateForPlayer($trainer, $playlist, $player);
        }

        usort($incomplete, static function (Playlist $a, Playlist $b) use ($dueDates): int {
            $dueA = $dueDates[(int) $a->getId()];
            $dueB = $dueDates[(int) $b->getId()];

            return match (true) {
                null === $dueA && null === $dueB => 0,
                null === $dueA => 1,
                null === $dueB => -1,
                default => $dueA <=> $dueB,
            };
        });

        return $incomplete;
    }

    /**
     * @return array<int, true>
     */
    private function completedContentItemIds(Trainer $trainer, PlayerProfile $player): array
    {
        $ids = [];
        foreach ($this->progressRepository->findForPlayer($trainer, $player) as $row) {
            if ($row->isCompleted()) {
                $ids[(int) $row->getContentItem()->getId()] = true;
            }
        }

        return $ids;
    }
}
