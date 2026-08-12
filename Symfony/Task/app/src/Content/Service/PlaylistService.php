<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Dto\VideoItemInput;
use App\Content\Entity\ContentItem;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistItem;
use App\Content\Repository\ContentItemRepository;
use App\Content\Repository\ContentUsageRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-04.01/04/09/10: create, edit, reorder, and delete Learn/Practice
 * playlists, and manage their items.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md
 */
final readonly class PlaylistService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlaylistRepository $playlists,
        private PlaylistItemRepository $playlistItems,
        private ContentItemRepository $contentItems,
        private ContentUsageRepository $contentUsage,
    ) {
    }

    /**
     * AC-04-1..3: a Learn playlist requires at least one video — videos are
     * always created inline/new here (see `VideoItemInput`'s own docblock),
     * never selected from an existing library.
     *
     * @param list<string>|null    $filterSkillLevels
     * @param list<string>|null    $filterPositions
     * @param list<string>|null    $filterAgeLevels
     * @param list<VideoItemInput> $videos
     */
    public function createLearnPlaylist(
        Trainer $trainer,
        string $title,
        ?string $description,
        ?array $filterSkillLevels,
        ?array $filterPositions,
        ?array $filterAgeLevels,
        bool $isPublic,
        string $audience,
        array $videos,
    ): Playlist {
        if ([] === $videos) {
            throw new \InvalidArgumentException('A Learn playlist requires at least one video.');
        }

        return $this->entityManager->wrapInTransaction(function () use (
            $trainer, $title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels, $isPublic, $audience, $videos,
        ): Playlist {
            $playlist = new Playlist($trainer, Playlist::PILLAR_LEARN, $title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels);
            $playlist->setVisibility($isPublic, $audience, new \DateTimeImmutable());
            $this->playlists->add($playlist);

            $order = 1;
            foreach ($videos as $video) {
                // AC-04-3: the same video may appear more than once, at
                // different positions — no dedup check here on purpose.
                $item = new ContentItem($trainer, Playlist::PILLAR_LEARN, $video->title, $video->youtubeUrl, $video->instructions, $video->durationSeconds, $video->tags);
                $this->contentItems->add($item);
                $this->playlistItems->add(new PlaylistItem($trainer, $playlist, $item, $order));
                ++$order;
            }

            $this->entityManager->flush();

            return $playlist;
        });
    }

    /**
     * AC-04-11: a Practice playlist's metadata. Drills are added afterward
     * via `addExistingItem()` — the same "add to playlist" path AC-04-8's
     * public-drill discovery uses, whether the drill is this trainer's own,
     * reused from another trainer's public library, or was just created
     * inline via `ContentItemService::createDrill()`. See the coder's final
     * report for why this is one flow rather than the API spec's literal
     * single-form framing.
     *
     * @param list<string>|null $filterSkillLevels
     * @param list<string>|null $filterPositions
     * @param list<string>|null $filterAgeLevels
     */
    public function createPracticePlaylist(
        Trainer $trainer,
        string $title,
        ?string $description,
        ?array $filterSkillLevels,
        ?array $filterPositions,
        ?array $filterAgeLevels,
        bool $isPublic,
        string $audience,
    ): Playlist {
        return $this->entityManager->wrapInTransaction(function () use (
            $trainer, $title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels, $isPublic, $audience,
        ): Playlist {
            $playlist = new Playlist($trainer, Playlist::PILLAR_PRACTICE, $title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels);
            $playlist->setVisibility($isPublic, $audience, new \DateTimeImmutable());
            $this->playlists->add($playlist);
            $this->entityManager->flush();

            return $playlist;
        });
    }

    /**
     * AC-04-31: title, description, filters. Already-assigned players see
     * the change immediately (no cache, no fan-out needed — the portal
     * reads the live row) and existing progress is untouched (ContentProgress
     * is keyed off ContentItem, never Playlist — BR-04-17 holds by
     * construction, not by anything this method does).
     *
     * AC-05-18: $priceUsdMinorUnits/$priceTokens ride the same save —
     * Playlist::updatePricing() is a separate entity method (so the two
     * concerns stay independently callable/testable) but this service
     * exposes one edit action, matching the one edit form
     * (PlaylistEditType).
     *
     * @param list<string>|null $filterSkillLevels
     * @param list<string>|null $filterPositions
     * @param list<string>|null $filterAgeLevels
     */
    public function updateDetails(
        Playlist $playlist,
        string $title,
        ?string $description,
        ?array $filterSkillLevels,
        ?array $filterPositions,
        ?array $filterAgeLevels,
        ?int $priceUsdMinorUnits = null,
        ?int $priceTokens = null,
    ): void {
        $this->entityManager->wrapInTransaction(function () use ($playlist, $title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels, $priceUsdMinorUnits, $priceTokens): void {
            $playlist->updateDetails($title, $description, $filterSkillLevels, $filterPositions, $filterAgeLevels);

            if (null !== $priceUsdMinorUnits && null !== $priceTokens) {
                $playlist->updatePricing($priceUsdMinorUnits, $priceTokens);
            }

            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-17/18/41, BR-04-3..5.
     */
    public function setVisibility(Playlist $playlist, bool $isPublic, string $audience): void
    {
        $this->entityManager->wrapInTransaction(function () use ($playlist, $isPublic, $audience): void {
            $playlist->setVisibility($isPublic, $audience, new \DateTimeImmutable());
            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-2: one more inline/new video, appended to the end.
     */
    public function addVideoItem(Playlist $playlist, VideoItemInput $video): PlaylistItem
    {
        return $this->entityManager->wrapInTransaction(function () use ($playlist, $video): PlaylistItem {
            $trainer = $playlist->getTrainer();
            $item = new ContentItem($trainer, Playlist::PILLAR_LEARN, $video->title, $video->youtubeUrl, $video->instructions, $video->durationSeconds, $video->tags);
            $this->contentItems->add($item);

            $playlistItem = new PlaylistItem($trainer, $playlist, $item, $this->playlistItems->nextSequenceOrder($playlist));
            $this->playlistItems->add($playlistItem);
            $this->entityManager->flush();

            return $playlistItem;
        });
    }

    /**
     * AC-04-8/AC-04-12, BR-04-12: adds an EXISTING content item (own or
     * another trainer's public one) by reference. Recording usage is
     * unconditional here — `ContentUsageRepository::recordFirstUse()`
     * itself no-ops when the item is the caller's own creation.
     */
    public function addExistingItem(Playlist $playlist, ContentItem $item, bool $isRequired = true, ?string $trainerNotes = null): PlaylistItem
    {
        return $this->entityManager->wrapInTransaction(function () use ($playlist, $item, $isRequired, $trainerNotes): PlaylistItem {
            $trainer = $playlist->getTrainer();
            $playlistItem = new PlaylistItem($trainer, $playlist, $item, $this->playlistItems->nextSequenceOrder($playlist), $isRequired, $trainerNotes);
            $this->playlistItems->add($playlistItem);
            $this->contentUsage->recordFirstUse($trainer, $item);
            $this->entityManager->flush();

            return $playlistItem;
        });
    }

    /**
     * AC-04-31/AC-04-33: removing an item never touches `ContentProgress` —
     * it has no foreign key to `PlaylistItem` at all, so a player's prior
     * progress on the removed item is preserved for history automatically.
     */
    public function removeItem(PlaylistItem $item): void
    {
        $this->entityManager->wrapInTransaction(function () use ($item): void {
            $this->playlistItems->remove($item);
            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-2/12: drag-and-drop reorder — one transaction, so the
     * DEFERRABLE INITIALLY DEFERRED unique index on
     * `(playlist_id, sequence_order)` never sees a transient collision
     * (e.g. swapping positions 1 and 2) mid-batch; PostgreSQL checks it only
     * at commit. AC-04-33: reordering never touches `ContentProgress`.
     *
     * @param list<int> $orderedItemIds
     */
    public function reorderItems(Playlist $playlist, array $orderedItemIds): void
    {
        $this->entityManager->wrapInTransaction(function () use ($playlist, $orderedItemIds): void {
            $byId = [];
            foreach ($this->playlistItems->findForPlaylist($playlist) as $item) {
                $byId[(int) $item->getId()] = $item;
            }

            $position = 1;
            foreach ($orderedItemIds as $id) {
                if (isset($byId[$id])) {
                    $byId[$id]->setSequenceOrder($position);
                    ++$position;
                }
            }

            $this->entityManager->flush();
        });
    }

    /**
     * AC-04-34, AC-04-36: soft delete only — the epic's own recommendation.
     * Other trainers' references (`PlaylistItem`/`ContentUsage` rows
     * pointing at this playlist's own items are unaffected; what breaks here
     * is OTHER trainers' `PlaylistItem` rows pointing AT one of this
     * playlist's content items, which is unrelated to the PLAYLIST being
     * deleted — playlists are never referenced by other playlists, only
     * content items are (BR-04-12 is stated for content, this method only
     * ever soft-deletes the playlist row).
     */
    public function delete(Playlist $playlist): void
    {
        $this->entityManager->wrapInTransaction(function () use ($playlist): void {
            $playlist->softDelete(new \DateTimeImmutable());
            $this->entityManager->flush();
        });
    }
}
