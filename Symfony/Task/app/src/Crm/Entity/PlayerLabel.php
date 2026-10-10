<?php

declare(strict_types=1);

namespace App\Crm\Entity;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Crm\Repository\PlayerLabelRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * Join row: one label applied to one player. BR-03-4: a player can have
 * multiple labels; removing a label from a player does not delete the
 * player. BR-03-5: deleting the `Label` itself cascades (`ON DELETE CASCADE`
 * on `label_id`) and removes it from every player it was applied to — that
 * cascade IS the rule, not a side effect this entity guards against.
 *
 * @see specs/database-designer-schema.md "`player_label`"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-4/5, AC-03-12, AC-03-28
 */
#[ORM\Entity(repositoryClass: PlayerLabelRepository::class)]
#[ORM\Table(name: 'player_label')]
#[ORM\UniqueConstraint(name: 'uniq_player_label_player_label', columns: ['player_id', 'label_id'])]
#[ORM\Index(name: 'idx_player_label_trainer_label', columns: ['trainer_id', 'label_id'])]
#[TrainerScoped]
class PlayerLabel
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

    #[ORM\ManyToOne(targetEntity: Label::class)]
    #[ORM\JoinColumn(name: 'label_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private Label $label;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'applied_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $appliedByAccount;

    #[ORM\Column(name: 'applied_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $appliedAt;

    public function __construct(Trainer $trainer, PlayerProfile $player, Label $label, Account $appliedByAccount)
    {
        $this->trainer = $trainer;
        $this->player = $player;
        $this->label = $label;
        $this->appliedByAccount = $appliedByAccount;
        $this->appliedAt = new \DateTimeImmutable();
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

    public function getLabel(): Label
    {
        return $this->label;
    }

    public function getAppliedByAccount(): Account
    {
        return $this->appliedByAccount;
    }

    public function getAppliedAt(): \DateTimeImmutable
    {
        return $this->appliedAt;
    }
}
