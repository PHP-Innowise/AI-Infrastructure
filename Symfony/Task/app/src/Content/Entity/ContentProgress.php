<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\ContentProgressRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * Keyed per (player, content item) — NOT per (player, playlist) — which is
 * what makes BR-04-17 (progress survives a playlist restructure or deletion)
 * true by construction rather than by extra bookkeeping: this row has no
 * foreign key to any `PlaylistAssignment` or even to a specific playlist at
 * all.
 *
 * BR-04-16: completion is automatic — the moment a player starts playback
 * (`recordEngagementStarted()`), never a manual "Mark as Complete" action.
 * The idempotent-replay guard (clicking play twice) lives in the SERVICE
 * (`ContentProgressService`), via a `WHERE completed_at IS NULL`-shaped
 * check before writing, matching the schema's own documented approach — this
 * entity's method is deliberately safe to call more than once regardless.
 *
 * @see specs/database-designer-schema.md "`content_progress`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-16/17, AC-04-25, AC-04-27..29
 */
#[ORM\Entity(repositoryClass: ContentProgressRepository::class)]
#[ORM\Table(name: 'content_progress')]
#[ORM\UniqueConstraint(name: 'uniq_content_progress_player_item', columns: ['player_id', 'content_item_id'])]
#[ORM\Index(name: 'idx_content_progress_trainer_player', columns: ['trainer_id', 'player_id'])]
#[TrainerScoped]
class ContentProgress
{
    public const STATUS_NOT_STARTED = 'not_started';
    public const STATUS_IN_PROGRESS = 'in_progress';
    public const STATUS_COMPLETED = 'completed';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\ManyToOne(targetEntity: ContentItem::class)]
    #[ORM\JoinColumn(name: 'content_item_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private ContentItem $contentItem;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_NOT_STARTED])]
    private string $status = self::STATUS_NOT_STARTED;

    #[ORM\Column(name: 'progress_percent', type: 'smallint', options: ['default' => 0])]
    private int $progressPercent = 0;

    #[ORM\Column(name: 'first_viewed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $firstViewedAt = null;

    #[ORM\Column(name: 'completed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $completedAt = null;

    #[ORM\Column(name: 'watch_time_seconds', type: 'integer', options: ['default' => 0])]
    private int $watchTimeSeconds = 0;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Trainer $trainer, PlayerProfile $player, ContentItem $contentItem)
    {
        $this->trainer = $trainer;
        $this->player = $player;
        $this->contentItem = $contentItem;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getContentItem(): ContentItem
    {
        return $this->contentItem;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isCompleted(): bool
    {
        return self::STATUS_COMPLETED === $this->status;
    }

    public function getProgressPercent(): int
    {
        return $this->progressPercent;
    }

    public function getFirstViewedAt(): ?\DateTimeImmutable
    {
        return $this->firstViewedAt;
    }

    public function getCompletedAt(): ?\DateTimeImmutable
    {
        return $this->completedAt;
    }

    public function getWatchTimeSeconds(): int
    {
        return $this->watchTimeSeconds;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    /**
     * BR-04-16, AC-04-25: "When player clicks play -> Content automatically
     * marked complete." Idempotent — a second call after completion is a
     * silent no-op (Testing Considerations: "Player clicks play on the same
     * video/drill more than once -> Completion stays idempotent"), so the
     * completion timestamp never moves once set.
     */
    public function recordEngagementStarted(\DateTimeImmutable $now): void
    {
        if (null === $this->firstViewedAt) {
            $this->firstViewedAt = $now;
        }

        if (self::STATUS_COMPLETED === $this->status) {
            return;
        }

        $this->status = self::STATUS_COMPLETED;
        $this->progressPercent = 100;
        $this->completedAt = $now;
        $this->updatedAt = $now;
    }
}
