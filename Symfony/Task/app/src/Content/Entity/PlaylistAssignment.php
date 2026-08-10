<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\PlaylistAssignmentRepository;
use App\Crm\Entity\Label;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * BR-04-13/14/15: one row per assignment ACT — to one player, one label
 * group, or one skill-level filter. Deliberately carries no per-player
 * status column: a group target covers an unbounded, membership-dependent
 * set of players, so per-player progress lives on `ContentProgress`
 * (keyed by player + content item) and this row's "who is covered" is
 * resolved at read time against CURRENT membership
 * (`ContentAssignmentResolver`), never frozen into a fan-out of rows at
 * assignment time. This is also what makes BR-04-15's "reassignment keeps
 * progress" true for free — `ContentProgress` never depended on this row.
 *
 * @see specs/database-designer-schema.md "`playlist_assignment`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-13..15, AC-04-13..16
 */
#[ORM\Entity(repositoryClass: PlaylistAssignmentRepository::class)]
#[ORM\Table(name: 'playlist_assignment')]
#[ORM\Index(name: 'idx_playlist_assignment_playlist', columns: ['playlist_id'])]
#[ORM\Index(name: 'idx_playlist_assignment_target_player', columns: ['target_player_id'])]
#[ORM\Index(name: 'idx_playlist_assignment_target_label', columns: ['target_label_id'])]
#[TrainerScoped]
class PlaylistAssignment
{
    public const TARGET_PLAYER = 'player';
    public const TARGET_LABEL = 'label';
    public const TARGET_SKILL_LEVEL = 'skill_level';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Playlist::class)]
    #[ORM\JoinColumn(name: 'playlist_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Playlist $playlist;

    #[ORM\Column(name: 'target_type', type: 'string', length: 16)]
    private string $targetType;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'target_player_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?PlayerProfile $targetPlayer = null;

    #[ORM\ManyToOne(targetEntity: Label::class)]
    #[ORM\JoinColumn(name: 'target_label_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Label $targetLabel = null;

    #[ORM\Column(name: 'target_skill_level', type: 'string', length: 50, nullable: true)]
    private ?string $targetSkillLevel = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'assigned_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $assignedByAccount;

    #[ORM\Column(name: 'assigned_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $assignedAt;

    #[ORM\Column(name: 'due_date', type: 'date_immutable', nullable: true)]
    private ?\DateTimeImmutable $dueDate = null;

    #[ORM\Column(type: 'text', nullable: true)]
    private ?string $note = null;

    public function __construct(
        Trainer $trainer,
        Playlist $playlist,
        string $targetType,
        Account $assignedByAccount,
        ?PlayerProfile $targetPlayer = null,
        ?Label $targetLabel = null,
        ?string $targetSkillLevel = null,
        ?\DateTimeImmutable $dueDate = null,
        ?string $note = null,
    ) {
        $this->guardTarget($targetType, $targetPlayer, $targetLabel, $targetSkillLevel);

        $this->trainer = $trainer;
        $this->playlist = $playlist;
        $this->targetType = $targetType;
        $this->targetPlayer = $targetPlayer;
        $this->targetLabel = $targetLabel;
        $this->targetSkillLevel = $targetSkillLevel;
        $this->assignedByAccount = $assignedByAccount;
        $this->assignedAt = new \DateTimeImmutable();
        $this->dueDate = $dueDate;
        $this->note = $note;
    }

    /**
     * @return list<string>
     */
    public static function targetTypes(): array
    {
        return [self::TARGET_PLAYER, self::TARGET_LABEL, self::TARGET_SKILL_LEVEL];
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

    public function getTargetType(): string
    {
        return $this->targetType;
    }

    public function getTargetPlayer(): ?PlayerProfile
    {
        return $this->targetPlayer;
    }

    public function getTargetLabel(): ?Label
    {
        return $this->targetLabel;
    }

    public function getTargetSkillLevel(): ?string
    {
        return $this->targetSkillLevel;
    }

    public function getAssignedByAccount(): Account
    {
        return $this->assignedByAccount;
    }

    public function getAssignedAt(): \DateTimeImmutable
    {
        return $this->assignedAt;
    }

    public function getDueDate(): ?\DateTimeImmutable
    {
        return $this->dueDate;
    }

    public function getNote(): ?string
    {
        return $this->note;
    }

    private function guardTarget(string $targetType, ?PlayerProfile $targetPlayer, ?Label $targetLabel, ?string $targetSkillLevel): void
    {
        if (!\in_array($targetType, self::targetTypes(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown assignment target type "%s".', $targetType));
        }

        $provided = array_filter([
            self::TARGET_PLAYER => null !== $targetPlayer,
            self::TARGET_LABEL => null !== $targetLabel,
            self::TARGET_SKILL_LEVEL => null !== $targetSkillLevel && '' !== trim($targetSkillLevel),
        ]);

        if (!isset($provided[$targetType]) || 1 !== \count($provided)) {
            throw new \InvalidArgumentException('Exactly one target (player, label, or skill level) must match the target type.');
        }
    }
}
