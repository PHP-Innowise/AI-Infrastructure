<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer's Stripe Connect status and pricing — split from `Trainer`
 * itself for exactly one reason (architect-architecture.md Decisions,
 * "`Trainer` registry placement"): `Trainer` is global and read
 * cross-tenant (the switcher, public content attribution, public form
 * pages); the Connect account id, fee rate and prices here must never be.
 *
 * **Identity through the foreign entity**: this table's primary key IS its
 * own tenant key (`trainer_id`), a 1-to-1-with-`Trainer` row with no
 * separate surrogate id — matching specs/database-designer-schema.md's own
 * note on this table exactly ("this table's PK *is* its own tenant key —
 * the 1-to-1-with-trainer pattern still satisfies Layer 1 literally").
 * `TenantFilter` (tenancy layer 2) still applies unmodified: it compares the
 * literal `trainer_id` column, which is this row's PK/FK either way.
 *
 * Created in the Billing (Epic-05) migration, not an Epic-01 one, despite
 * the schema doc filing its own column table under an "Identity module"
 * heading — see Version20260811100000's own docblock for why.
 *
 * @see specs/database-designer-schema.md "`trainer_billing_settings`"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md BR-05-1, AC-05-1, AC-05-2, AC-05-27
 */
#[ORM\Entity(repositoryClass: TrainerBillingSettingsRepository::class)]
#[ORM\Table(name: 'trainer_billing_settings')]
#[TrainerScoped]
class TrainerBillingSettings
{
    public const ONBOARDING_PENDING = 'pending';
    public const ONBOARDING_COMPLETE = 'complete';
    public const ONBOARDING_INCOMPLETE = 'incomplete';

    public const PAYOUT_MONTHLY = 'monthly';
    public const PAYOUT_WEEKLY = 'weekly';

    /**
     * AC-05-2: 5% default, Super-Admin-editable per trainer (AC-05-27).
     */
    public const DEFAULT_FEE_BASIS_POINTS = 500;

    /**
     * BR-05-13: $15/month default.
     */
    public const DEFAULT_MONTHLY_SUBSCRIPTION_MINOR_UNITS = 1500;

    /**
     * BR-05-1: illustrative default ($1 = 100 minor units, so $10/token).
     */
    public const DEFAULT_TOKEN_PRICE_MINOR_UNITS = 1000;

    #[ORM\Id]
    #[ORM\OneToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'stripe_connect_account_id', type: 'string', length: 255, nullable: true, unique: true)]
    private ?string $stripeConnectAccountId = null;

    #[ORM\Column(name: 'stripe_connect_onboarding_status', type: 'string', length: 16, options: ['default' => self::ONBOARDING_PENDING])]
    private string $stripeConnectOnboardingStatus = self::ONBOARDING_PENDING;

    #[ORM\Column(name: 'platform_fee_basis_points', type: 'smallint', options: ['default' => self::DEFAULT_FEE_BASIS_POINTS])]
    private int $platformFeeBasisPoints = self::DEFAULT_FEE_BASIS_POINTS;

    #[ORM\Column(name: 'monthly_subscription_price_minor_units', type: 'integer', options: ['default' => self::DEFAULT_MONTHLY_SUBSCRIPTION_MINOR_UNITS])]
    private int $monthlySubscriptionPriceMinorUnits = self::DEFAULT_MONTHLY_SUBSCRIPTION_MINOR_UNITS;

    #[ORM\Column(name: 'token_price_minor_units', type: 'integer', options: ['default' => self::DEFAULT_TOKEN_PRICE_MINOR_UNITS])]
    private int $tokenPriceMinorUnits = self::DEFAULT_TOKEN_PRICE_MINOR_UNITS;

    /**
     * BR-05-14: set only once the trainer enables Player Subscriptions.
     */
    #[ORM\Column(name: 'player_subscription_price_minor_units', type: 'integer', nullable: true)]
    private ?int $playerSubscriptionPriceMinorUnits = null;

    #[ORM\Column(name: 'payout_schedule', type: 'string', length: 16, options: ['default' => self::PAYOUT_MONTHLY])]
    private string $payoutSchedule = self::PAYOUT_MONTHLY;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
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

    public function getStripeConnectAccountId(): ?string
    {
        return $this->stripeConnectAccountId;
    }

    public function getStripeConnectOnboardingStatus(): string
    {
        return $this->stripeConnectOnboardingStatus;
    }

    /**
     * AC-05-1: "Stripe Connected ✓" once onboarding completes. AC-05-3:
     * paid events/content are blocked until this is true.
     */
    public function isStripeConnected(): bool
    {
        return self::ONBOARDING_COMPLETE === $this->stripeConnectOnboardingStatus;
    }

    public function getPlatformFeeBasisPoints(): int
    {
        return $this->platformFeeBasisPoints;
    }

    public function getMonthlySubscriptionPriceMinorUnits(): int
    {
        return $this->monthlySubscriptionPriceMinorUnits;
    }

    public function getTokenPriceMinorUnits(): int
    {
        return $this->tokenPriceMinorUnits;
    }

    public function getPlayerSubscriptionPriceMinorUnits(): ?int
    {
        return $this->playerSubscriptionPriceMinorUnits;
    }

    public function getPayoutSchedule(): string
    {
        return $this->payoutSchedule;
    }

    /**
     * US-05.01: called once Stripe redirects back and the platform verifies
     * onboarding against Stripe's own Account object (never trusted from the
     * redirect alone).
     */
    public function attachStripeConnectAccount(string $stripeConnectAccountId): void
    {
        $this->stripeConnectAccountId = $stripeConnectAccountId;
        $this->touch();
    }

    public function updateOnboardingStatus(string $status): void
    {
        if (!\in_array($status, [self::ONBOARDING_PENDING, self::ONBOARDING_COMPLETE, self::ONBOARDING_INCOMPLETE], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown Stripe Connect onboarding status "%s".', $status));
        }

        $this->stripeConnectOnboardingStatus = $status;
        $this->touch();
    }

    /**
     * AC-05-27/28: Super Admin edits the subscription amount and/or fee
     * rate. The caller (the fee-edit service) is responsible for the
     * audit-log entry and for the "subscription rate applies next cycle"
     * distinction — this only stores the new values.
     */
    public function updatePricing(int $monthlySubscriptionPriceMinorUnits, int $platformFeeBasisPoints): void
    {
        if ($platformFeeBasisPoints < 0 || $platformFeeBasisPoints > 10000) {
            throw new \InvalidArgumentException('The platform fee rate must be between 0 and 10000 basis points.');
        }

        if ($monthlySubscriptionPriceMinorUnits < 0) {
            throw new \InvalidArgumentException('The monthly subscription price cannot be negative.');
        }

        $this->monthlySubscriptionPriceMinorUnits = $monthlySubscriptionPriceMinorUnits;
        $this->platformFeeBasisPoints = $platformFeeBasisPoints;
        $this->touch();
    }

    /**
     * BR-05-1: the trainer's own $/token price, and the token-package
     * bundle discounts editable from `billing_trainer_pricing_edit`.
     */
    public function updateTokenPrice(int $tokenPriceMinorUnits): void
    {
        if ($tokenPriceMinorUnits <= 0) {
            throw new \InvalidArgumentException('The token price must be greater than zero.');
        }

        $this->tokenPriceMinorUnits = $tokenPriceMinorUnits;
        $this->touch();
    }

    /**
     * BR-05-14: the trainer sets this when enabling Player Subscriptions.
     */
    public function updatePlayerSubscriptionPrice(?int $priceMinorUnits): void
    {
        if (null !== $priceMinorUnits && $priceMinorUnits <= 0) {
            throw new \InvalidArgumentException('The player subscription price must be greater than zero.');
        }

        $this->playerSubscriptionPriceMinorUnits = $priceMinorUnits;
        $this->touch();
    }

    public function updatePayoutSchedule(string $schedule): void
    {
        if (!\in_array($schedule, [self::PAYOUT_MONTHLY, self::PAYOUT_WEEKLY], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown payout schedule "%s".', $schedule));
        }

        $this->payoutSchedule = $schedule;
        $this->touch();
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
