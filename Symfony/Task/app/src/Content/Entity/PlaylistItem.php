<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\PlaylistItemRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One content item's position inside one playlist. Ordinary trainer-scoped
 * join row (unlike `Playlist`/`ContentItem`, this one carries no publication
 * widening of its own — a playlist's own read predicate already governs
 * whether the whole playlist, and therefore this row, is visible at all).
 *
 * `trainer` is the PLAYLIST OWNER'S tenant, not the referenced content
 * item's creator — BR-04-12: adding another trainer's public drill stores a
 * reference (this row), never a copy, and the reference lives in the
 * REUSING trainer's own tenant.
 *
 * No unique constraint on (playlist, contentItem): the same video may
 * legitimately appear at more than one position (US-04.01 "Validation" edge
 * case, AC-04-3).
 *
 * @see specs/database-designer-schema.md "`playlist_item`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-12, AC-04-2, AC-04-12
 */
#[ORM\Entity(repositoryClass: PlaylistItemRepository::class)]
#[ORM\Table(name: 'playlist_item')]
#[ORM\Index(name: 'idx_playlist_item_content', columns: ['content_item_id'])]
#[TrainerScoped]
class PlaylistItem
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Playlist::class)]
    #[ORM\JoinColumn(name: 'playlist_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private Playlist $playlist;

    /**
     * RESTRICT, not CASCADE: `content_item` is soft-deleted, so this FK never
     * actually fires on a delete — it is what lets BR-04-12's "reference
     * breaks and shows Unavailable" work without an orphaned row. See
     * `ContentItem::softDelete()`.
     */
    #[ORM\ManyToOne(targetEntity: ContentItem::class)]
    #[ORM\JoinColumn(name: 'content_item_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private ContentItem $contentItem;

    #[ORM\Column(name: 'sequence_order', type: 'integer')]
    private int $sequenceOrder;

    #[ORM\Column(name: 'is_required', type: 'boolean', options: ['default' => true])]
    private bool $isRequired = true;

    /**
     * AC-04-12: trainer notes and an estimated-time override, specific to
     * THIS playlist's use of the item — never written back onto the shared
     * `ContentItem` row, which may be referenced by other playlists (this
     * trainer's own, or another trainer's entirely).
     */
    #[ORM\Column(name: 'trainer_notes', type: 'text', nullable: true)]
    private ?string $trainerNotes = null;

    public function __construct(
        Trainer $trainer,
        Playlist $playlist,
        ContentItem $contentItem,
        int $sequenceOrder,
        bool $isRequired = true,
        ?string $trainerNotes = null,
    ) {
        $this->trainer = $trainer;
        $this->playlist = $playlist;
        $this->contentItem = $contentItem;
        $this->sequenceOrder = $sequenceOrder;
        $this->isRequired = $isRequired;
        $this->trainerNotes = $trainerNotes;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getPlaylist(): Playlist
    {
        return $this->playlist;
    }

    public function getContentItem(): ContentItem
    {
        return $this->contentItem;
    }

    public function getSequenceOrder(): int
    {
        return $this->sequenceOrder;
    }

    public function setSequenceOrder(int $sequenceOrder): void
    {
        $this->sequenceOrder = $sequenceOrder;
    }

    public function isRequired(): bool
    {
        return $this->isRequired;
    }

    public function getTrainerNotes(): ?string
    {
        return $this->trainerNotes;
    }

    /**
     * AC-04-12: per-playlist notes and time override, edited independently
     * of reordering.
     */
    public function updateContext(bool $isRequired, ?string $trainerNotes): void
    {
        $this->isRequired = $isRequired;
        $this->trainerNotes = $trainerNotes;
    }
}
