<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\EntitlementCoverageRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Entity\Rsvp;
use Doctrine\ORM\Mapping as ORM;

/**
 * Which entitlement covered which RSVP — implements BR-05-14's "1 advance
 * booking per day" rule and AC-02-63. Spending against a
 * `SubscriptionEntitlement` decrements nothing and writes no `TokenEntry`;
 * this row is the only record that an RSVP was entitlement-funded
 * (architect-architecture.md "Subscriptions are entitlements, not ledger
 * entries").
 *
 * `rsvpId` is UNIQUE: an RSVP is covered by at most one entitlement.
 *
 * @see specs/database-designer-schema.md "`entitlement_coverage`"
 * @see specs/architect-architecture.md "Subscriptions are entitlements, not ledger entries"
 */
#[ORM\Entity(repositoryClass: EntitlementCoverageRepository::class)]
#[ORM\Table(name: 'entitlement_coverage')]
#[TrainerScoped]
class EntitlementCoverage
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: SubscriptionEntitlement::class)]
    #[ORM\JoinColumn(name: 'subscription_entitlement_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private SubscriptionEntitlement $subscriptionEntitlement;

    #[ORM\ManyToOne(targetEntity: Rsvp::class)]
    #[ORM\JoinColumn(name: 'rsvp_id', referencedColumnName: 'id', nullable: false, unique: true, onDelete: 'RESTRICT')]
    private Rsvp $rsvp;

    #[ORM\Column(name: 'covered_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $coveredAt;

    public function __construct(Trainer $trainer, SubscriptionEntitlement $subscriptionEntitlement, Rsvp $rsvp, ?\DateTimeImmutable $coveredAt = null)
    {
        $this->trainer = $trainer;
        $this->subscriptionEntitlement = $subscriptionEntitlement;
        $this->rsvp = $rsvp;
        $this->coveredAt = $coveredAt ?? new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getSubscriptionEntitlement(): SubscriptionEntitlement
    {
        return $this->subscriptionEntitlement;
    }

    public function getRsvp(): Rsvp
    {
        return $this->rsvp;
    }

    public function getCoveredAt(): \DateTimeImmutable
    {
        return $this->coveredAt;
    }
}
