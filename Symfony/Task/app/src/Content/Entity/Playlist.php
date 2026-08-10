<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\PlaylistRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\Mapping as ORM;

/**
 * A Learn or Practice sequence of content items a trainer builds and a
 * player pays to unlock (A8). Learn and Practice are "functionally
 * identical... differing only in their label" per the epic itself (line 16
 * of the source), which is exactly why this is one entity with a `pillar`
 * discriminator column rather than two classes.
 *
 * **Deliberately NOT `#[TrainerScoped]`.** Every other trainer-scoped entity
 * in this codebase carries that attribute so Doctrine's tenant filter (layer
 * 2) adds a blanket `trainer_id = :tenant` predicate to every query — correct
 * for entities with no cross-tenant read. `Playlist` is the one declared
 * exception (architect-architecture.md "The publication exception"): a row
 * that has ever been published must remain readable by every tenant, and the
 * filter's only shape is strict equality with no widened variant. Applying
 * `#[TrainerScoped]` here would silently re-narrow every query back to
 * "my tenant only", defeating the whole point of `everPublishedAt` and the
 * widened RLS SELECT policy underneath it. Every repository method on
 * `PlaylistRepository` therefore states its own tenant predicate explicitly
 * (own-only vs. accessible/published) rather than relying on the automatic
 * filter; RLS (layer 5) remains the authoritative floor beneath all of them,
 * bounding a forgotten predicate to "shows more published content than
 * intended", never a true cross-tenant leak of a still-private row (see the
 * RLS policy in the Epic-04 migration).
 *
 * Ownership never transfers (BR-04-11): `trainer` is always the ORIGINAL
 * creator and never reassigned by anything in this class.
 *
 * @see specs/database-designer-schema.md "`playlist`"
 * @see specs/architect-architecture.md "The publication exception"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-3..5, AC-04-1, AC-04-11, AC-04-41
 */
#[ORM\Entity(repositoryClass: PlaylistRepository::class)]
#[ORM\Table(name: 'playlist')]
#[ORM\Index(name: 'idx_playlist_trainer_deleted', columns: ['trainer_id', 'deleted_at'])]
class Playlist
{
    public const PILLAR_LEARN = 'learn';
    public const PILLAR_PRACTICE = 'practice';

    public const AUDIENCE_PLAYERS_AND_COACHES = 'players_and_coaches';
    public const AUDIENCE_COACHES_ONLY = 'coaches_only';

    public const MAX_TITLE_LENGTH = 100;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 255)]
    private string $title;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $description = null;

    #[ORM\Column(type: 'string', length: 16)]
    private string $pillar;

    #[ORM\Column(name: 'is_public', type: 'boolean', options: ['default' => false])]
    private bool $isPublic = false;

    /**
     * BR-04-5, "The publication exception": set once, the first time this
     * playlist becomes public, and never cleared again — including when the
     * trainer later reverts to private. This is the column the widened RLS
     * SELECT policy compares, not `isPublic`.
     */
    #[ORM\Column(name: 'ever_published_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $everPublishedAt = null;

    #[ORM\Column(type: 'string', length: 24, options: ['default' => self::AUDIENCE_PLAYERS_AND_COACHES])]
    private string $audience = self::AUDIENCE_PLAYERS_AND_COACHES;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(name: 'filter_skill_levels', type: 'text_array', nullable: true)]
    private ?array $filterSkillLevels = null;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(name: 'filter_positions', type: 'text_array', nullable: true)]
    private ?array $filterPositions = null;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(name: 'filter_age_levels', type: 'text_array', nullable: true)]
    private ?array $filterAgeLevels = null;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    #[ORM\Column(name: 'deleted_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $deletedAt = null;

    /**
     * @param list<string>|null $filterSkillLevels
     * @param list<string>|null $filterPositions
     * @param list<string>|null $filterAgeLevels
     */
    public function __construct(
        Trainer $trainer,
        string $pillar,
        string $title,
        ?string $description = null,
        ?array $filterSkillLevels = null,
        ?array $filterPositions = null,
        ?array $filterAgeLevels = null,
    ) {
        $this->guardPillar($pillar);
        $this->guardTitle($title);

        $this->trainer = $trainer;
        $this->pillar = $pillar;
        $this->title = $title;
        $this->description = $description;
        $this->filterSkillLevels = $filterSkillLevels;
        $this->filterPositions = $filterPositions;
        $this->filterAgeLevels = $filterAgeLevels;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    /**
     * @return list<string>
     */
    public static function pillars(): array
    {
        return [self::PILLAR_LEARN, self::PILLAR_PRACTICE];
    }

    /**
     * @return list<string>
     */
    public static function audiences(): array
    {
        return [self::AUDIENCE_PLAYERS_AND_COACHES, self::AUDIENCE_COACHES_ONLY];
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getTitle(): string
    {
        return $this->title;
    }

    public function getDescription(): ?string
    {
        return $this->description;
    }

    public function getPillar(): string
    {
        return $this->pillar;
    }

    public function isPublic(): bool
    {
        return $this->isPublic;
    }

    public function getEverPublishedAt(): ?\DateTimeImmutable
    {
        return $this->everPublishedAt;
    }

    /**
     * The widening fact itself: has this row EVER been public, regardless of
     * its CURRENT state. Mirrors the RLS predicate exactly.
     */
    public function hasEverBeenPublished(): bool
    {
        return null !== $this->everPublishedAt;
    }

    public function getAudience(): string
    {
        return $this->audience;
    }

    public function isCoachesOnly(): bool
    {
        return self::AUDIENCE_COACHES_ONLY === $this->audience;
    }

    /**
     * @return list<string>|null
     */
    public function getFilterSkillLevels(): ?array
    {
        return $this->filterSkillLevels;
    }

    /**
     * @return list<string>|null
     */
    public function getFilterPositions(): ?array
    {
        return $this->filterPositions;
    }

    /**
     * @return list<string>|null
     */
    public function getFilterAgeLevels(): ?array
    {
        return $this->filterAgeLevels;
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
     * AC-04-31: title, description, and customization filters. Pillar and
     * ownership are deliberately not editable here — Learn vs. Practice is a
     * creation-time choice (US-04.01/US-04.04 are two distinct entry points),
     * and ownership never transfers (BR-04-11).
     *
     * @param list<string>|null $filterSkillLevels
     * @param list<string>|null $filterPositions
     * @param list<string>|null $filterAgeLevels
     */
    public function updateDetails(
        string $title,
        ?string $description,
        ?array $filterSkillLevels,
        ?array $filterPositions,
        ?array $filterAgeLevels,
    ): void {
        $this->guardTitle($title);

        $this->title = $title;
        $this->description = $description;
        $this->filterSkillLevels = $filterSkillLevels;
        $this->filterPositions = $filterPositions;
        $this->filterAgeLevels = $filterAgeLevels;
        $this->touch();
    }

    /**
     * AC-04-17/AC-04-41, BR-04-3..5: sets BOTH orthogonal facts at once —
     * publication (public library or not) and in-tenant audience (players
     * and coaches, or coaches only) — per architect-architecture.md
     * "Playlist visibility storage". The fourth combination (published AND
     * coaches-only) is never a real state ("internal certifications content
     * shared across every trainer" — the migration's own CHECK constraint
     * name for this), so it is rejected here too, not only at the database.
     *
     * `everPublishedAt` is a one-way ratchet: reverting `isPublic` to false
     * never clears it, which is the entire mechanism behind BR-04-5's
     * "existing references keep working."
     */
    public function setVisibility(bool $public, string $audience, \DateTimeImmutable $now): void
    {
        $this->guardAudience($audience);

        if ($public && self::AUDIENCE_COACHES_ONLY === $audience) {
            throw new \InvalidArgumentException('Public, coaches-only content is not a valid combination — coach-only content is never discoverable by other trainers.');
        }

        $this->isPublic = $public;
        $this->audience = $audience;

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

    private function guardTitle(string $title): void
    {
        if ('' === trim($title)) {
            throw new \InvalidArgumentException('A playlist requires a non-empty title.');
        }

        if (mb_strlen($title) > self::MAX_TITLE_LENGTH) {
            throw new \InvalidArgumentException(sprintf('A playlist title cannot exceed %d characters.', self::MAX_TITLE_LENGTH));
        }
    }

    private function guardPillar(string $pillar): void
    {
        if (!\in_array($pillar, self::pillars(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown playlist pillar "%s".', $pillar));
        }
    }

    private function guardAudience(string $audience): void
    {
        if (!\in_array($audience, self::audiences(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown playlist audience "%s".', $audience));
        }
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
