<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\ContentItemRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\Mapping as ORM;

/**
 * A single video or drill — the atomic unit `PlaylistItem` sequences into a
 * `Playlist`. `type = 'video'` rows hydrate as this base class directly;
 * `type = 'drill'` rows hydrate as `Drill` (joined-table inheritance), which
 * carries the Practice-pillar-specific columns in `drill_detail`.
 *
 * **Deliberately NOT `#[TrainerScoped]`**, for exactly the same reason as
 * `Playlist` — see that class's own docblock. `everPublishedAt` is the same
 * one-way "has ever been published" widening fact.
 *
 * BR-04-11: `trainer` is the ORIGINAL creator and never reassigned — public
 * content reused by another trainer is referenced (`PlaylistItem`,
 * `ContentUsage`), never copied, so this column never changes after
 * creation.
 *
 * @see specs/database-designer-schema.md "`content_item`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-1/2/3/4/5/11/12
 */
#[ORM\Entity(repositoryClass: ContentItemRepository::class)]
#[ORM\Table(name: 'content_item')]
#[ORM\InheritanceType('JOINED')]
#[ORM\DiscriminatorColumn(name: 'type', type: 'string', length: 16)]
#[ORM\DiscriminatorMap(['video' => ContentItem::class, 'drill' => Drill::class])]
#[ORM\Index(name: 'idx_content_item_trainer_deleted', columns: ['trainer_id', 'deleted_at'])]
class ContentItem
{
    public const TYPE_VIDEO = 'video';
    public const TYPE_DRILL = 'drill';

    public const MAX_TITLE_LENGTH = 100;
    public const MAX_INSTRUCTIONS_LENGTH = 1000;

    /**
     * AC-04-3/AC-04-6: a plain, deliberately permissive format check — any
     * youtube.com/youtu.be URL, not a strict video-id pattern. The epic asks
     * for "valid-format" validation, not YouTube API verification (BR-04-1:
     * the platform never calls out to YouTube to confirm a link resolves).
     */
    public const YOUTUBE_URL_PATTERN = '#^https?://(www\.)?(youtube\.com/(watch\?v=|shorts/)|youtu\.be/)[\w-]+#i';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    protected ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    protected Trainer $trainer;

    #[ORM\Column(type: 'string', length: 16)]
    protected string $pillar;

    #[ORM\Column(type: 'string', length: 100)]
    protected string $title;

    #[ORM\Column(type: 'text', nullable: true)]
    protected ?string $instructions = null;

    #[ORM\Column(name: 'youtube_url', type: 'string', length: 2048)]
    protected string $youtubeUrl;

    #[ORM\Column(name: 'duration_seconds', type: 'integer', nullable: true)]
    protected ?int $durationSeconds = null;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(type: 'text_array', nullable: true)]
    protected ?array $tags = null;

    #[ORM\Column(name: 'is_public', type: 'boolean', options: ['default' => false])]
    protected bool $isPublic = false;

    #[ORM\Column(name: 'ever_published_at', type: 'datetimetz_immutable', nullable: true)]
    protected ?\DateTimeImmutable $everPublishedAt = null;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    protected \DateTimeImmutable $createdAt;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    protected \DateTimeImmutable $updatedAt;

    #[ORM\Column(name: 'deleted_at', type: 'datetimetz_immutable', nullable: true)]
    protected ?\DateTimeImmutable $deletedAt = null;

    /**
     * @param list<string>|null $tags
     */
    public function __construct(
        Trainer $trainer,
        string $pillar,
        string $title,
        string $youtubeUrl,
        ?string $instructions = null,
        ?int $durationSeconds = null,
        ?array $tags = null,
    ) {
        $this->guardPillar($pillar);
        $this->guardTitle($title);
        $this->guardInstructions($instructions);
        $this->guardYoutubeUrl($youtubeUrl);

        $this->trainer = $trainer;
        $this->pillar = $pillar;
        $this->title = $title;
        $this->youtubeUrl = $youtubeUrl;
        $this->instructions = $instructions;
        $this->durationSeconds = $durationSeconds;
        $this->tags = $tags;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    /**
     * @return list<string>
     */
    public static function types(): array
    {
        return [self::TYPE_VIDEO, self::TYPE_DRILL];
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    /**
     * The discriminator's own value for this instance — `video` for the base
     * class, overridden by `Drill`.
     */
    public function getType(): string
    {
        return self::TYPE_VIDEO;
    }

    public function getPillar(): string
    {
        return $this->pillar;
    }

    public function getTitle(): string
    {
        return $this->title;
    }

    public function getInstructions(): ?string
    {
        return $this->instructions;
    }

    public function getYoutubeUrl(): string
    {
        return $this->youtubeUrl;
    }

    public function getDurationSeconds(): ?int
    {
        return $this->durationSeconds;
    }

    /**
     * @return list<string>|null
     */
    public function getTags(): ?array
    {
        return $this->tags;
    }

    public function isPublic(): bool
    {
        return $this->isPublic;
    }

    public function getEverPublishedAt(): ?\DateTimeImmutable
    {
        return $this->everPublishedAt;
    }

    public function hasEverBeenPublished(): bool
    {
        return null !== $this->everPublishedAt;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function getDeletedAt(): ?\DateTimeImmutable
    {
        return $this->deletedAt;
    }

    public function isDeleted(): bool
    {
        return null !== $this->deletedAt;
    }

    /**
     * @param list<string>|null $tags
     */
    public function updateCommonFields(
        string $title,
        string $youtubeUrl,
        ?string $instructions,
        ?int $durationSeconds,
        ?array $tags,
    ): void {
        $this->guardTitle($title);
        $this->guardInstructions($instructions);
        $this->guardYoutubeUrl($youtubeUrl);

        $this->title = $title;
        $this->youtubeUrl = $youtubeUrl;
        $this->instructions = $instructions;
        $this->durationSeconds = $durationSeconds;
        $this->tags = $tags;
        $this->touch();
    }

    /**
     * AC-04-17/BR-04-3..5: same one-way "has ever been published" mechanism
     * as `Playlist::setVisibility()` — see that method's own docblock.
     * `ContentItem`/`Drill` carry no coach-only audience concept (that is a
     * `Playlist`-level field only, per AC-04-41's own scope note), so this
     * takes a plain public/private toggle.
     */
    public function setPublic(bool $public, \DateTimeImmutable $now): void
    {
        $this->isPublic = $public;

        if ($public && null === $this->everPublishedAt) {
            $this->everPublishedAt = $now;
        }

        $this->touch();
    }

    public function softDelete(\DateTimeImmutable $now): void
    {
        $this->deletedAt = $now;
        $this->touch();
    }

    protected function guardTitle(string $title): void
    {
        if ('' === trim($title)) {
            throw new \InvalidArgumentException('A content item requires a non-empty title.');
        }

        if (mb_strlen($title) > self::MAX_TITLE_LENGTH) {
            throw new \InvalidArgumentException(sprintf('A content item title cannot exceed %d characters.', self::MAX_TITLE_LENGTH));
        }
    }

    protected function guardInstructions(?string $instructions): void
    {
        if (null !== $instructions && mb_strlen($instructions) > self::MAX_INSTRUCTIONS_LENGTH) {
            throw new \InvalidArgumentException(sprintf('Instructions cannot exceed %d characters.', self::MAX_INSTRUCTIONS_LENGTH));
        }
    }

    /**
     * AC-04-3/AC-04-6, "Edge cases" (invalid-format YouTube URL rejected).
     */
    protected function guardYoutubeUrl(string $youtubeUrl): void
    {
        if (1 !== preg_match(self::YOUTUBE_URL_PATTERN, $youtubeUrl)) {
            throw new \InvalidArgumentException('A valid YouTube URL is required (youtube.com/watch, youtube.com/shorts, or youtu.be).');
        }
    }

    private function guardPillar(string $pillar): void
    {
        if (!\in_array($pillar, Playlist::pillars(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown content pillar "%s".', $pillar));
        }
    }

    protected function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
