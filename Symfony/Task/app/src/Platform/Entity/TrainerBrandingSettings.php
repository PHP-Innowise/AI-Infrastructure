<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Platform\Repository\TrainerBrandingSettingsRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer's logo and primary brand colour. Global and public by nature —
 * rendered on unauthenticated camp/ShareLink landing pages, not only inside
 * the authenticated portal (BR-08-6).
 *
 * 1-to-1 extension of Trainer: `trainer_id` is simultaneously this table's
 * primary key and its foreign key, the shared PK/FK pattern already used by
 * AccountProfile.
 *
 * @see specs/database-designer-schema.md "`trainer_branding_settings`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-60..63
 */
#[ORM\Entity(repositoryClass: TrainerBrandingSettingsRepository::class)]
#[ORM\Table(name: 'trainer_branding_settings')]
class TrainerBrandingSettings
{
    #[ORM\Id]
    #[ORM\OneToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', onDelete: 'CASCADE')]
    private Trainer $trainer;

    /**
     * AC-01-60: PNG/JPG/SVG, max 2MB, rendered via a plain <img>, never
     * inlined — a relative path under public/uploads, not the binary itself.
     */
    #[ORM\Column(name: 'logo_path', type: 'string', length: 2048, nullable: true)]
    private ?string $logoPath = null;

    #[ORM\Column(name: 'primary_color_hex', type: 'string', length: 7, nullable: true)]
    private ?string $primaryColorHex = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Trainer $trainer)
    {
        $this->trainer = $trainer;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getLogoPath(): ?string
    {
        return $this->logoPath;
    }

    public function getPrimaryColorHex(): ?string
    {
        return $this->primaryColorHex;
    }

    /**
     * AC-01-62: applies immediately for the whole organization, because
     * branding is read fresh per-request — there is nothing to invalidate.
     */
    public function updateLogo(?string $logoPath): void
    {
        $this->logoPath = $logoPath;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function updatePrimaryColor(?string $hex): void
    {
        if (null !== $hex && 1 !== preg_match('/^#[0-9A-Fa-f]{6}$/', $hex)) {
            throw new \InvalidArgumentException('primaryColorHex must be a 6-digit hex colour, e.g. #00B300.');
        }

        $this->primaryColorHex = $hex;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * AC-01-61: reset-to-default option.
     */
    public function resetPrimaryColor(): void
    {
        $this->primaryColorHex = null;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
