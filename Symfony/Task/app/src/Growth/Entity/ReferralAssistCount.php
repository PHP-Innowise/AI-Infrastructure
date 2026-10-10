<?php

declare(strict_types=1);

namespace App\Growth\Entity;

use App\Growth\Repository\ReferralAssistCountRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * One running count per (player, trainer) — BR-06-7: assists are tracked per
 * player-trainer relationship, never globally. Resets to 0 (never deleted)
 * each time it reaches the platform-wide threshold and a reward fires
 * (BR-06-5); a Super Admin ratio change preserves whatever count is already
 * here (AC-06-30) since nothing about this row changes on a rule edit —
 * only the threshold it is compared against, read fresh at increment time.
 *
 * @see specs/database-designer-schema.md "`referral_assist_count`"
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-9..11, BR-06-4..7
 */
#[ORM\Entity(repositoryClass: ReferralAssistCountRepository::class)]
#[ORM\Table(name: 'referral_assist_count')]
#[ORM\UniqueConstraint(name: 'uniq_referral_assist_count_trainer_player', columns: ['trainer_id', 'player_id'])]
#[TrainerScoped]
class ReferralAssistCount
{
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

    #[ORM\Column(name: 'assist_count', type: 'integer', options: ['default' => 0])]
    private int $assistCount = 0;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Trainer $trainer, PlayerProfile $player)
    {
        $this->trainer = $trainer;
        $this->player = $player;
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

    public function getAssistCount(): int
    {
        return $this->assistCount;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    /**
     * BR-06-5: one assist per qualifying referral purchase.
     */
    public function increment(): void
    {
        ++$this->assistCount;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * BR-06-5: "the counter resets and repeats" once a reward fires.
     */
    public function resetAfterReward(): void
    {
        $this->assistCount = 0;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
