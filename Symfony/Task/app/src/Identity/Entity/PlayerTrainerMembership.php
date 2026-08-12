<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\PlayerTrainerMembershipRepository;
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
#[ORM\Entity(repositoryClass: PlayerTrainerMembershipRepository::class)]
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

    /**
     * BR-01-27: which ShareLink was used to connect them, if any (not every
     * source is code-driven — coach_invite and camp_registration are not).
     */
    #[ORM\ManyToOne(targetEntity: ShareLink::class)]
    #[ORM\JoinColumn(name: 'share_link_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?ShareLink $shareLink = null;

    #[ORM\Column(name: 'joined_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $joinedAt;

    /**
     * AC-01-24: set when the parent removes the child from this trainer.
     * Soft-removal — the row (and its history) survives.
     */
    #[ORM\Column(name: 'removed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $removedAt = null;

    public function __construct(Trainer $trainer, PlayerProfile $player, string $source, ?ShareLink $shareLink = null)
    {
        if (!\in_array($source, self::sources(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown association source "%s".', $source));
        }

        $this->trainer = $trainer;
        $this->player = $player;
        $this->source = $source;
        $this->shareLink = $shareLink;
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

    public function getShareLink(): ?ShareLink
    {
        return $this->shareLink;
    }

    public function getSkillLevel(): ?string
    {
        return $this->skillLevel;
    }

    public function setSkillLevel(?string $skillLevel): void
    {
        $this->skillLevel = $skillLevel;
    }

    public function getJoinedAt(): \DateTimeImmutable
    {
        return $this->joinedAt;
    }

    public function getRemovedAt(): ?\DateTimeImmutable
    {
        return $this->removedAt;
    }

    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }

    /**
     * AC-01-24: disassociates the child from this trainer. Soft: the row and
     * its history remain, `status` moves to inactive and `removedAt` records
     * when. The caller is responsible for the "cancels upcoming RSVPs"
     * fan-out once Scheduling exists (Epic-02) — out of Epic-01's reach.
     */
    public function remove(): void
    {
        $this->status = self::STATUS_INACTIVE;
        $this->removedAt = new \DateTimeImmutable();
    }

    /**
     * AC-01-13/BR-01-12: re-joining a trainer reactivates the same row
     * instead of a second insert — "no duplicate account... only a new
     * trainer association" extended to the membership row itself.
     */
    public function reactivate(string $source, ?ShareLink $shareLink = null): void
    {
        if (!\in_array($source, self::sources(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown association source "%s".', $source));
        }

        $this->status = self::STATUS_ACTIVE;
        $this->source = $source;
        $this->shareLink = $shareLink;
        $this->joinedAt = new \DateTimeImmutable();
        $this->removedAt = null;
    }
}
