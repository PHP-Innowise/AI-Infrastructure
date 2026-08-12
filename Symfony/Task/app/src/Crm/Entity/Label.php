<?php

declare(strict_types=1);

namespace App\Crm\Entity;

use App\Crm\Repository\LabelRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer-defined tag used to organize players into meaningful groups.
 *
 * BR-03-3: names collide case-insensitively within one trainer ("Elite" and
 * "elite" are the same label) but not across trainers (BR-03-4). The
 * case-folded comparison lives at the database layer only — `name_normalized`
 * is a `GENERATED ALWAYS AS (lower(name)) STORED` column backing a unique
 * index (see the migration) — and is deliberately NOT mapped as a Doctrine
 * property: nothing in the domain ever reads it directly, and
 * `LabelRepository::findOneByTrainerAndNameCaseInsensitive()` does the same
 * comparison via DQL's `LOWER()` for the pre-check, with the database
 * constraint as the authoritative backstop under concurrency (the same
 * "named unique constraint remains authoritative" pattern used throughout
 * this codebase, e.g. `MembershipService`).
 *
 * @see specs/database-designer-schema.md "`label`"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-3/4/5, AC-03-11..14
 */
#[ORM\Entity(repositoryClass: LabelRepository::class)]
#[ORM\Table(name: 'label')]
#[TrainerScoped]
class Label
{
    public const MAX_NAME_LENGTH = 50;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: self::MAX_NAME_LENGTH)]
    private string $name;

    #[ORM\Column(name: 'color_hex', type: 'string', length: 7)]
    private string $colorHex;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Trainer $trainer, string $name, string $colorHex)
    {
        $this->trainer = $trainer;
        $this->guardName($name);
        $this->guardColorHex($colorHex);

        $this->name = $name;
        $this->colorHex = $colorHex;
        $this->createdAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getName(): string
    {
        return $this->name;
    }

    public function getColorHex(): string
    {
        return $this->colorHex;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    /**
     * AC-03-13: edit name and/or color.
     */
    public function rename(string $name, string $colorHex): void
    {
        $this->guardName($name);
        $this->guardColorHex($colorHex);

        $this->name = $name;
        $this->colorHex = $colorHex;
    }

    private function guardName(string $name): void
    {
        if ('' === trim($name)) {
            throw new \InvalidArgumentException('A label requires a non-empty name.');
        }

        if (mb_strlen($name) > self::MAX_NAME_LENGTH) {
            throw new \InvalidArgumentException(sprintf('A label name cannot exceed %d characters.', self::MAX_NAME_LENGTH));
        }
    }

    private function guardColorHex(string $colorHex): void
    {
        if (1 !== preg_match('/^#[0-9A-Fa-f]{6}$/', $colorHex)) {
            throw new \InvalidArgumentException('A label color must be a 6-digit hex code, e.g. "#00B300".');
        }
    }
}
