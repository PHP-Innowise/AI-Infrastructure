<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\DrillRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\Mapping as ORM;

/**
 * The Practice-pillar-specific extension of `ContentItem`, joined on
 * `drill_detail.id` (shared PK — Doctrine Class Table Inheritance). US-04.02.
 *
 * The shared PK/FK column is named `id`, NOT `content_item_id` as
 * `specs/database-designer-schema.md` literally names it — Doctrine ORM
 * 3.6 (the version pinned in this project's `composer.lock`) has no
 * `PrimaryKeyJoinColumn` mapping attribute at all (confirmed absent from
 * `vendor/doctrine/orm` entirely) to rename a JOINED-inheritance child
 * table's identifier column; `getIdentifierColumnNames()` always reuses the
 * SAME column name inherited from the parent's own `#[ORM\Id]` field
 * (`id`), unconditionally. Naming the column `content_item_id` as specified
 * would generate `SELECT ... FROM drill_detail WHERE id = ?`-shaped SQL
 * against a column that does not exist — a real runtime failure, not a
 * cosmetic mismatch — the first time any code loads or persists a `Drill`.
 * Recorded here and in the coder's final report as a genuine
 * spec-vs-framework-capability conflict, same class of deviation as
 * `app.current_trainer` (not `_trainer_id`) elsewhere in this codebase: the
 * underlying intent (shared PK/FK, `ON DELETE CASCADE`) is preserved
 * exactly, only the column's own name differs from the schema doc's literal
 * text.
 *
 * `drill_detail` carries no `trainer_id` of its own and no RLS policy — see
 * the Epic-04 migration's own docblock for why: every real query reaches it
 * through its parent `content_item` row, whose widened publication policy
 * already governs readability.
 *
 * @see specs/database-designer-schema.md "`drill_detail`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-18/19, AC-04-4..6
 */
#[ORM\Entity(repositoryClass: DrillRepository::class)]
#[ORM\Table(name: 'drill_detail')]
class Drill extends ContentItem
{
    public const DIFFICULTY_BEGINNER = 'beginner';
    public const DIFFICULTY_INTERMEDIATE = 'intermediate';
    public const DIFFICULTY_ADVANCED = 'advanced';
    public const DIFFICULTY_ELITE = 'elite';

    public const SPACE_SMALL = 'small';
    public const SPACE_MEDIUM = 'medium';
    public const SPACE_LARGE = 'large';

    #[ORM\Column(name: 'difficulty_level', type: 'string', length: 16)]
    private string $difficultyLevel;

    /**
     * @var list<string>|null
     */
    #[ORM\Column(type: 'text_array', nullable: true)]
    private ?array $equipment = null;

    #[ORM\Column(name: 'space_requirement', type: 'string', length: 16, nullable: true)]
    private ?string $spaceRequirement = null;

    #[ORM\Column(name: 'player_count', type: 'string', length: 20, nullable: true)]
    private ?string $playerCount = null;

    #[ORM\Column(name: 'duration_min_minutes', type: 'integer', nullable: true)]
    private ?int $durationMinMinutes = null;

    #[ORM\Column(name: 'duration_max_minutes', type: 'integer', nullable: true)]
    private ?int $durationMaxMinutes = null;

    /**
     * AC-04-6: "requires at least one category" — enforced in the
     * constructor/updateDrillFields(), never empty despite being nullable at
     * the column level (the column is nullable only because a hypothetical
     * future non-drill joined subclass could otherwise never share this
     * table shape; every real `Drill` row always has at least one).
     *
     * @var list<string>
     */
    #[ORM\Column(type: 'text_array', nullable: true)]
    private ?array $categories = null;

    /**
     * @param list<string>|null $tags
     * @param list<string>|null $equipment
     * @param list<string>      $categories
     */
    public function __construct(
        Trainer $trainer,
        string $title,
        string $youtubeUrl,
        string $difficultyLevel,
        array $categories,
        ?string $instructions = null,
        ?array $tags = null,
        ?array $equipment = null,
        ?string $spaceRequirement = null,
        ?string $playerCount = null,
        ?int $durationMinMinutes = null,
        ?int $durationMaxMinutes = null,
    ) {
        // A Drill is always Practice-pillar (Data Requirements, epic line
        // 583: "Drill (Practice-pillar-specific fields)") — never passed in,
        // never a choice the trainer makes.
        parent::__construct($trainer, Playlist::PILLAR_PRACTICE, $title, $youtubeUrl, $instructions, null, $tags);

        $this->guardDifficulty($difficultyLevel);
        $this->guardCategories($categories);
        $this->guardDurationRange($durationMinMinutes, $durationMaxMinutes);

        $this->difficultyLevel = $difficultyLevel;
        $this->categories = $categories;
        $this->equipment = $equipment;
        $this->spaceRequirement = $spaceRequirement;
        $this->playerCount = $playerCount;
        $this->durationMinMinutes = $durationMinMinutes;
        $this->durationMaxMinutes = $durationMaxMinutes;
    }

    /**
     * @return list<string>
     */
    public static function difficultyLevels(): array
    {
        return [self::DIFFICULTY_BEGINNER, self::DIFFICULTY_INTERMEDIATE, self::DIFFICULTY_ADVANCED, self::DIFFICULTY_ELITE];
    }

    /**
     * @return list<string>
     */
    public static function spaceRequirements(): array
    {
        return [self::SPACE_SMALL, self::SPACE_MEDIUM, self::SPACE_LARGE];
    }

    public function getType(): string
    {
        return self::TYPE_DRILL;
    }

    public function getDifficultyLevel(): string
    {
        return $this->difficultyLevel;
    }

    /**
     * @return list<string>|null
     */
    public function getEquipment(): ?array
    {
        return $this->equipment;
    }

    public function getSpaceRequirement(): ?string
    {
        return $this->spaceRequirement;
    }

    public function getPlayerCount(): ?string
    {
        return $this->playerCount;
    }

    public function getDurationMinMinutes(): ?int
    {
        return $this->durationMinMinutes;
    }

    public function getDurationMaxMinutes(): ?int
    {
        return $this->durationMaxMinutes;
    }

    /**
     * @return list<string>
     */
    public function getCategories(): array
    {
        return $this->categories ?? [];
    }

    /**
     * AC-04-4: the drill-specific fields, edited independently of the common
     * ContentItem fields (updateCommonFields()) — a controller calls both.
     *
     * @param list<string>      $categories
     * @param list<string>|null $equipment
     */
    public function updateDrillFields(
        string $difficultyLevel,
        array $categories,
        ?array $equipment,
        ?string $spaceRequirement,
        ?string $playerCount,
        ?int $durationMinMinutes,
        ?int $durationMaxMinutes,
    ): void {
        $this->guardDifficulty($difficultyLevel);
        $this->guardCategories($categories);
        $this->guardDurationRange($durationMinMinutes, $durationMaxMinutes);

        $this->difficultyLevel = $difficultyLevel;
        $this->categories = $categories;
        $this->equipment = $equipment;
        $this->spaceRequirement = $spaceRequirement;
        $this->playerCount = $playerCount;
        $this->durationMinMinutes = $durationMinMinutes;
        $this->durationMaxMinutes = $durationMaxMinutes;
        $this->touch();
    }

    private function guardDifficulty(string $difficultyLevel): void
    {
        if (!\in_array($difficultyLevel, self::difficultyLevels(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown drill difficulty "%s".', $difficultyLevel));
        }
    }

    /**
     * AC-04-6: "requires at least one category."
     *
     * @param list<string> $categories
     */
    private function guardCategories(array $categories): void
    {
        if ([] === array_filter($categories, static fn (string $c): bool => '' !== trim($c))) {
            throw new \InvalidArgumentException('A drill requires at least one category.');
        }
    }

    private function guardDurationRange(?int $min, ?int $max): void
    {
        if (null !== $min && null !== $max && $max < $min) {
            throw new \InvalidArgumentException('A drill\'s maximum duration cannot be less than its minimum.');
        }
    }
}
