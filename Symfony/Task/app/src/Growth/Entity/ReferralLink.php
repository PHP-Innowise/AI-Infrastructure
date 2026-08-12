<?php

declare(strict_types=1);

namespace App\Growth\Entity;

use App\Growth\Repository\ReferralLinkRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * AC-06-1: every player automatically has exactly one referral link per
 * trainer they train with — no "generate" action, always active, never
 * expires. The URL itself (`platform.com/join/{trainer-slug}/{player-id}`)
 * is composed at render time from `trainer.getSlug()` and this row's own
 * player id; there is no separate stored code column, since nothing about
 * the URL is meant to be secret (`specs/database-designer-schema.md`
 * "`referral_link`" — "no separate stored code column is needed since
 * nothing about the URL is secret or random").
 *
 * Provisioned lazily, on first access, by `ReferralLinkService::linkFor()`
 * — the same "get-or-create on read" idiom
 * `TrainerBillingSettingsRepository::getOrCreateForTrainer()` and
 * `PlatformConfigurationRepository::getOrCreate()` already establish, rather
 * than requiring a hook into Identity's membership-creation flow (Growth
 * must not write Identity's entities, and the module map does not authorize
 * Identity to call into Growth).
 *
 * @see specs/database-designer-schema.md "`referral_link`"
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-1..3
 */
#[ORM\Entity(repositoryClass: ReferralLinkRepository::class)]
#[ORM\Table(name: 'referral_link')]
#[ORM\UniqueConstraint(name: 'uniq_referral_link_trainer_player', columns: ['trainer_id', 'player_id'])]
#[TrainerScoped]
class ReferralLink
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    /**
     * The referrer — AC-06-3: one link per (player, trainer) pair, not
     * globally per player.
     */
    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Trainer $trainer, PlayerProfile $player)
    {
        $this->trainer = $trainer;
        $this->player = $player;
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

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }
}
