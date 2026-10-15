<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\PlatformSubscriptionRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\Mapping as ORM;

/**
 * The platform's own billing relationship with a trainer (BR-05-13: $15/mo
 * to the platform owner, via Stripe Billing). Global, not trainer-scoped —
 * architect-architecture.md "Entity population — Global": the Epic-07
 * "active trainers" dashboard count reads `status` directly, with no
 * crossing read needed, and this table carries no confidential figure
 * (status and a Stripe id only — see this class's own docblock on why no
 * money value lives here).
 *
 * No money figure is stored here, per the earnings boundary — the
 * subscription AMOUNT actually charged lives in Stripe (and, per-trainer,
 * in `trainer_billing_settings.monthlySubscriptionPriceMinorUnits` as the
 * rate currently in force); this table tracks only whether the
 * relationship is healthy.
 *
 * @see specs/database-designer-schema.md "`platform_subscription` — Billing"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md BR-05-13
 */
#[ORM\Entity(repositoryClass: PlatformSubscriptionRepository::class)]
#[ORM\Table(name: 'platform_subscription')]
#[ORM\UniqueConstraint(name: 'uniq_platform_subscription_trainer', columns: ['trainer_id'])]
#[ORM\Index(name: 'idx_platform_subscription_status', columns: ['status'])]
class PlatformSubscription
{
    public const STATUS_PENDING = 'pending';
    public const STATUS_ACTIVE = 'active';
    public const STATUS_PAST_DUE = 'past_due';
    public const STATUS_SUSPENDED = 'suspended';
    public const STATUS_CANCELED = 'canceled';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'stripe_subscription_id', type: 'string', length: 255, nullable: true)]
    private ?string $stripeSubscriptionId = null;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_PENDING])]
    private string $status = self::STATUS_PENDING;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Trainer $trainer)
    {
        $this->trainer = $trainer;
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

    public function getStripeSubscriptionId(): ?string
    {
        return $this->stripeSubscriptionId;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    /**
     * BR-07-7's "active trainers = active Stripe subscription" reads this
     * directly.
     */
    public function isActive(): bool
    {
        return self::STATUS_ACTIVE === $this->status;
    }

    public function attachStripeSubscription(string $stripeSubscriptionId): void
    {
        $this->stripeSubscriptionId = $stripeSubscriptionId;
        $this->touch();
    }

    /**
     * BR-05-13: trainer creation provisions this Pending; the first
     * successful Stripe Billing invoice moves it Active. On repeated
     * failure (3 retries over 10 days) it moves Suspended — "cannot create
     * paid events" — until payment succeeds, at which point it is
     * reactivated.
     */
    public function updateStatus(string $status): void
    {
        $statuses = [self::STATUS_PENDING, self::STATUS_ACTIVE, self::STATUS_PAST_DUE, self::STATUS_SUSPENDED, self::STATUS_CANCELED];

        if (!\in_array($status, $statuses, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown platform subscription status "%s".', $status));
        }

        $this->status = $status;
        $this->touch();
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
