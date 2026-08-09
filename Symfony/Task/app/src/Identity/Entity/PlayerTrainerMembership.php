<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A player's association with one trainer.
 *
 * Trainer-scoped: this is where the tenancy boundary actually bites. One
 * player may appear under several trainers, and neither trainer may see the
 * other's row — which is why `trainer_id` is carried denormalized here (layer
 * 1), the Doctrine filter applies (layer 2), and the table carries an RLS
 * policy (layer 5).
 *
 * AC-03-68 requires every association to record how it came about, and owner
 * decision A3 added a fourth source, `camp_registration`.
 *
 * @see specs/database-designer-schema.md "`player_trainer_membership`"
 */
#[ORM\Entity]
#[ORM\Table(name: 'player_trainer_membership')]
#[ORM\UniqueConstraint(name: 'uniq_player_trainer', columns: ['player_profile_id', 'trainer_id'])]
#[ORM\Index(name: 'idx_ptm_trainer', columns: ['trainer_id'])]
#[TrainerScoped]
class PlayerTrainerMembership
{
    public const SOURCE_SHARELINK = 'sharelink';
    public const SOURCE_EVENT_REGISTRATION = 'event_registration';
    public const SOURCE_COACH_INVITE = 'coach_invite';
    public const SOURCE_CAMP_REGISTRATION = 'camp_registration';

    public const STATUS_ACTIVE = 'active';
    public const STATUS_INACTIVE = 'inactive';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    /**
     * The denormalized tenant key. Tenancy layer 1: every trainer-scoped table
     * carries it so the policy is a plain column comparison rather than a join.
     */
    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_profile_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\Column(type: 'string', length: 32)]
    private string $source;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_ACTIVE])]
    private string $status = self::STATUS_ACTIVE;

    #[ORM\Column(name: 'skill_level', type: 'string', length: 50, nullable: true)]
    private ?string $skillLevel = null;

    #[ORM\Column(name: 'joined_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $joinedAt;

    public function __construct(Trainer $trainer, PlayerProfile $player, string $source)
    {
        if (!\in_array($source, self::sources(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown association source "%s".', $source));
        }

        $this->trainer = $trainer;
        $this->player = $player;
        $this->source = $source;
        $this->joinedAt = new \DateTimeImmutable();
    }

    /**
     * @return list<string>
     */
    public static function sources(): array
    {
        return [
            self::SOURCE_SHARELINK,
            self::SOURCE_EVENT_REGISTRATION,
            self::SOURCE_COACH_INVITE,
            self::SOURCE_CAMP_REGISTRATION,
        ];
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

    public function getSource(): string
    {
        return $this->source;
    }

    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }
}
