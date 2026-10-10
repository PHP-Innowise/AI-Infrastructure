<?php

declare(strict_types=1);

namespace App\Growth\Entity;

use App\Growth\Repository\CouponRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer's discount code. BR-06-8..11.
 *
 * Codes are **case-sensitive** — the epic is silent on case, and the schema
 * chose the conservative default (`specs/database-designer-schema.md`
 * "`coupon`": "See Open questions — case-sensitivity unstated"). This
 * differs deliberately from `Label.name`, which collides
 * case-insensitively (BR-03-3) — do not "fix" this into
 * case-insensitivity without reopening that spec decision.
 *
 * @see specs/database-designer-schema.md "`coupon`"
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-17..28, BR-06-8..11
 */
#[ORM\Entity(repositoryClass: CouponRepository::class)]
#[ORM\Table(name: 'coupon')]
#[ORM\UniqueConstraint(name: 'uniq_coupon_trainer_code', columns: ['trainer_id', 'code'])]
#[TrainerScoped]
class Coupon
{
    public const DISCOUNT_PERCENTAGE = 'percentage';
    public const DISCOUNT_FIXED = 'fixed';

    public const APPLIES_TO_EVENTS = 'events';
    public const APPLIES_TO_CONTENT = 'content';
    public const APPLIES_TO_BOTH = 'both';

    /**
     * Q-06.10's default: eligibility is a per-coupon setting (the superset
     * reading, which keeps the field) — `specs/architect-architecture.md`
     * Decisions, "Q-06.10 default".
     */
    public const ELIGIBILITY_ANY_PLAYER = 'any_player';
    public const ELIGIBILITY_NEW_PLAYERS_ONLY = 'new_players_only';

    private const CODE_PATTERN = '/^[A-Za-z0-9-]{4,20}$/';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 50)]
    private string $code;

    #[ORM\Column(name: 'discount_type', type: 'string', length: 16)]
    private string $discountType;

    /**
     * Percentage points (1-100) or minor currency units, depending on
     * $discountType.
     */
    #[ORM\Column(name: 'discount_value', type: 'integer')]
    private int $discountValue;

    #[ORM\Column(name: 'applies_to', type: 'string', length: 16)]
    private string $appliesTo;

    #[ORM\Column(name: 'usage_limit', type: 'integer', nullable: true)]
    private ?int $usageLimit = null;

    #[ORM\Column(name: 'usage_count', type: 'integer', options: ['default' => 0])]
    private int $usageCount = 0;

    #[ORM\Column(type: 'string', length: 24, options: ['default' => self::ELIGIBILITY_ANY_PLAYER])]
    private string $eligibility = self::ELIGIBILITY_ANY_PLAYER;

    #[ORM\Column(name: 'expires_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $expiresAt = null;

    #[ORM\Column(name: 'is_active', type: 'boolean', options: ['default' => true])]
    private bool $isActive = true;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'created_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $createdByAccount;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(
        Trainer $trainer,
        string $code,
        string $discountType,
        int $discountValue,
        string $appliesTo,
        ?int $usageLimit,
        string $eligibility,
        ?\DateTimeImmutable $expiresAt,
        Account $createdByAccount,
        bool $isActive = true,
    ) {
        self::guardCode($code);
        self::guardDiscountTypeAndValue($discountType, $discountValue);
        self::guardAppliesTo($appliesTo);
        self::guardEligibility($eligibility);
        self::guardUsageLimit($usageLimit);
        self::guardExpiresAtIsFuture($expiresAt);

        $this->trainer = $trainer;
        $this->code = $code;
        $this->discountType = $discountType;
        $this->discountValue = $discountValue;
        $this->appliesTo = $appliesTo;
        $this->usageLimit = $usageLimit;
        $this->eligibility = $eligibility;
        $this->isActive = $isActive;
        $this->expiresAt = $expiresAt;
        $this->createdByAccount = $createdByAccount;
        $this->createdAt = new \DateTimeImmutable();
    }

    // --- AC-06-18 validation, shared by the constructor -----------------

    public static function guardCode(string $code): void
    {
        if (1 !== preg_match(self::CODE_PATTERN, $code)) {
            throw new \InvalidArgumentException('AC-06-18: a coupon code must be 4-20 characters, alphanumeric plus hyphens.');
        }
    }

    public static function guardDiscountTypeAndValue(string $discountType, int $discountValue): void
    {
        if (!\in_array($discountType, [self::DISCOUNT_PERCENTAGE, self::DISCOUNT_FIXED], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown coupon discount type "%s".', $discountType));
        }

        if ($discountValue <= 0) {
            throw new \InvalidArgumentException('AC-06-18: the discount value must be greater than 0.');
        }

        if (self::DISCOUNT_PERCENTAGE === $discountType && $discountValue > 100) {
            throw new \InvalidArgumentException('AC-06-18: a percentage discount must be between 1 and 100.');
        }
    }

    public static function guardAppliesTo(string $appliesTo): void
    {
        if (!\in_array($appliesTo, [self::APPLIES_TO_EVENTS, self::APPLIES_TO_CONTENT, self::APPLIES_TO_BOTH], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown coupon applies-to scope "%s".', $appliesTo));
        }
    }

    public static function guardEligibility(string $eligibility): void
    {
        if (!\in_array($eligibility, [self::ELIGIBILITY_ANY_PLAYER, self::ELIGIBILITY_NEW_PLAYERS_ONLY], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown coupon eligibility rule "%s".', $eligibility));
        }
    }

    public static function guardUsageLimit(?int $usageLimit): void
    {
        if (null !== $usageLimit && $usageLimit <= 0) {
            throw new \InvalidArgumentException('A coupon usage limit, if set, must be a positive integer.');
        }
    }

    /**
     * AC-06-18: "an expiration date, if set, must be a future date."
     * Application-level only — a Postgres CHECK constraint cannot reference
     * `now()` (not immutable), so this cannot be enforced in the schema.
     */
    public static function guardExpiresAtIsFuture(?\DateTimeImmutable $expiresAt, ?\DateTimeImmutable $now = null): void
    {
        if (null === $expiresAt) {
            return;
        }

        if ($expiresAt <= ($now ?? new \DateTimeImmutable())) {
            throw new \InvalidArgumentException('AC-06-18: a coupon expiration date must be in the future.');
        }
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getCode(): string
    {
        return $this->code;
    }

    public function getDiscountType(): string
    {
        return $this->discountType;
    }

    public function getDiscountValue(): int
    {
        return $this->discountValue;
    }

    public function getAppliesTo(): string
    {
        return $this->appliesTo;
    }

    public function getUsageLimit(): ?int
    {
        return $this->usageLimit;
    }

    public function getUsageCount(): int
    {
        return $this->usageCount;
    }

    public function getEligibility(): string
    {
        return $this->eligibility;
    }

    public function getExpiresAt(): ?\DateTimeImmutable
    {
        return $this->expiresAt;
    }

    public function isActive(): bool
    {
        return $this->isActive;
    }

    public function getCreatedByAccount(): Account
    {
        return $this->createdByAccount;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function isExpired(\DateTimeImmutable $now): bool
    {
        return null !== $this->expiresAt && $this->expiresAt <= $now;
    }

    public function isUsageLimitReached(): bool
    {
        return null !== $this->usageLimit && $this->usageCount >= $this->usageLimit;
    }

    /**
     * BR-06-8: active, unexpired, under its usage limit. Eligibility
     * (Q-06.10) is a separate, player-specific check —
     * `CouponEligibilityChecker`'s own job, not this method's.
     */
    public function isRedeemable(\DateTimeImmutable $now): bool
    {
        return $this->isActive && !$this->isExpired($now) && !$this->isUsageLimitReached();
    }

    public function appliesToPurchaseType(string $purchaseType): bool
    {
        return self::APPLIES_TO_BOTH === $this->appliesTo || $this->appliesTo === $purchaseType;
    }

    /**
     * AC-06-25: increments usage and auto-deactivates once the limit is
     * reached — called once per confirmed redemption
     * (`CouponRedemptionRepository`'s own unique `payment_record_id`
     * constraint is what keeps this from double-counting a webhook
     * redelivery).
     */
    public function recordRedemption(): void
    {
        ++$this->usageCount;

        if ($this->isUsageLimitReached()) {
            $this->isActive = false;
        }
    }

    /**
     * AC-06-27: edit is scoped to expiration, usage limit, and status only
     * — code, discount type/value, applies-to, and eligibility are
     * immutable after creation (not stated as editable by any AC; the code
     * a trainer has already shared publicly must keep meaning the same
     * discount).
     */
    public function updateEditableFields(?\DateTimeImmutable $expiresAt, ?int $usageLimit, bool $isActive): void
    {
        self::guardUsageLimit($usageLimit);
        self::guardExpiresAtIsFuture($expiresAt);

        $this->expiresAt = $expiresAt;
        $this->usageLimit = $usageLimit;
        $this->isActive = $isActive;
    }

    public function deactivate(): void
    {
        $this->isActive = false;
    }

    public function hasBeenUsed(): bool
    {
        return $this->usageCount > 0;
    }
}
