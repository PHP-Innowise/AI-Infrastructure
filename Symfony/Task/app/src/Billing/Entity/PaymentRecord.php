<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\PaymentRecordRepository;
use App\Content\Entity\Playlist;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Entity\Rsvp;
use Doctrine\ORM\Mapping as ORM;

/**
 * Mirror of a Stripe object, kept MUTABLE — unlike `TokenEntry`, which is
 * append-only — because it reconciles to Stripe's own lifecycle
 * (architect-architecture.md "Payment records").
 *
 * **Idempotency**: the id is assigned (INSERT) BEFORE the Stripe call, and
 * `StripeGateway` derives its idempotency key from that id — "a timeout
 * retry reuses the key." This means a row exists in `pending` status ahead
 * of the Stripe round-trip for every type; the webhook (or, for a token
 * spend, the same synchronous call) then updates `status` and the
 * `stripe*`/`related*` columns as the outcome becomes known.
 *
 * **No trainer earnings/net/payout figure lives here or anywhere else
 * locally** (architect-architecture.md "The earnings boundary") — this
 * table stores what the platform needs operationally (the charge amount,
 * the fee passed to Stripe as `application_fee_amount`), never a figure a
 * future screen could present as revenue.
 *
 * **Refunds are new rows, never mutations** of the row being refunded — see
 * `refundsPaymentRecordId`. The original row's own `status` may still move
 * to `refunded` (it mirrors Stripe's lifecycle), but "Refunded" as displayed
 * in transaction history is derived from the refund row's existence.
 *
 * @see specs/database-designer-schema.md "`payment_record`"
 * @see specs/architect-architecture.md "Payment records", "Money and the platform fee"
 */
#[ORM\Entity(repositoryClass: PaymentRecordRepository::class)]
#[ORM\Table(name: 'payment_record')]
#[ORM\Index(name: 'idx_payment_record_trainer_payer_created', columns: ['trainer_id', 'payer_account_id', 'created_at'])]
#[TrainerScoped]
class PaymentRecord
{
    public const TYPE_TOKEN_PURCHASE = 'token_purchase';
    public const TYPE_EVENT_RSVP = 'event_rsvp';
    public const TYPE_CONTENT_PURCHASE = 'content_purchase';
    /**
     * Unlike every other type, `player_subscription` has no `related_*_id`
     * column of its own. The other four types each point at a target that
     * already exists before the payment (a token package, an RSVP's event,
     * a playlist, a form) or is created independent of this row (a camp
     * registration's form submission); a subscription purchase instead
     * CREATES its `SubscriptionEntitlement` from the payment, and that
     * entitlement already carries a required, unique `payment_record_id`
     * pointing back here. A `related_subscription_entitlement_id` column
     * was tried and removed — it duplicated that same fact in the reverse
     * direction, was never read or written outside this entity, and made
     * `subscription_entitlement` and `payment_record` reference each other
     * (the same shape that forces `related_rsvp_id` into a hand-written,
     * `DEFERRABLE INITIALLY DEFERRED` migration constraint) for no
     * information gained. `getRelatedSubscriptionEntitlement()` does not
     * exist here — nothing in this codebase needs to navigate
     * payment-record-to-entitlement; only the entitlement's own
     * `getPaymentRecord()` (the required, unique direction) is ever used.
     */
    public const TYPE_PLAYER_SUBSCRIPTION = 'player_subscription';
    public const TYPE_CAMP_REGISTRATION = 'camp_registration';

    public const METHOD_TOKEN = 'token';
    public const METHOD_CARD = 'card';

    public const STATUS_PENDING = 'pending';
    public const STATUS_COMPLETED = 'completed';
    public const STATUS_FAILED = 'failed';
    public const STATUS_REFUNDED = 'refunded';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 20)]
    private string $type;

    /**
     * Nullable — A3/A4: a camp payer may have no account (Epic-08, not built
     * in this codebase; the column exists so `payment_record`'s shape does
     * not need to change once it is).
     */
    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'payer_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $payerAccount = null;

    #[ORM\Column(name: 'contact_name', type: 'string', length: 255)]
    private string $contactName;

    #[ORM\Column(name: 'contact_email', type: 'string', length: 255, columnDefinition: 'CITEXT NOT NULL')]
    private string $contactEmail;

    #[ORM\Column(name: 'contact_phone', type: 'string', length: 32, nullable: true)]
    private ?string $contactPhone = null;

    #[ORM\Column(name: 'payment_method', type: 'string', length: 8)]
    private string $paymentMethod;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_PENDING])]
    private string $status = self::STATUS_PENDING;

    #[ORM\Column(name: 'amount_minor_units', type: 'integer')]
    private int $amountMinorUnits;

    /**
     * Snapshotted at creation from `trainer_billing_settings`, never
     * recomputed — "Money and the platform fee — Where it runs". NULL on a
     * refund row (the fee was already charged on the original).
     */
    #[ORM\Column(name: 'fee_rate_basis_points', type: 'smallint', nullable: true)]
    private ?int $feeRateBasisPoints = null;

    #[ORM\Column(name: 'platform_fee_minor_units', type: 'integer', options: ['default' => 0])]
    private int $platformFeeMinorUnits = 0;

    #[ORM\Column(name: 'stripe_payment_intent_id', type: 'string', length: 255, nullable: true)]
    private ?string $stripePaymentIntentId = null;

    #[ORM\Column(name: 'stripe_charge_id', type: 'string', length: 255, nullable: true)]
    private ?string $stripeChargeId = null;

    #[ORM\Column(name: 'stripe_refund_id', type: 'string', length: 255, nullable: true)]
    private ?string $stripeRefundId = null;

    #[ORM\ManyToOne(targetEntity: self::class)]
    #[ORM\JoinColumn(name: 'refunds_payment_record_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?self $refundsPaymentRecord = null;

    #[ORM\ManyToOne(targetEntity: TokenPackage::class)]
    #[ORM\JoinColumn(name: 'related_token_package_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?TokenPackage $relatedTokenPackage = null;

    /**
     * `ON DELETE NO ACTION`, not `RESTRICT` — deliberate, and not
     * interchangeable here. `rsvp` and `payment_record` reference each
     * other (this column and `rsvp.payment_record_id`), so this constraint
     * is `DEFERRABLE INITIALLY DEFERRED` in the migration DDL (Doctrine's
     * ORM attributes cannot express DEFERRABLE, so the real migration SQL
     * is hand-written — this mapping exists for schema-diff/documentation
     * parity, not as the source of truth). Postgres silently ignores
     * DEFERRABLE for the delete-triggered check when the action is
     * `RESTRICT` — only `NO ACTION` actually honors the deferred timing.
     * Confirmed directly: `doctrine:fixtures:load`'s `ORMPurger` deletes
     * every table in one transaction, and a `RESTRICT` here still raised
     * `SQLSTATE[23503]` synchronously on the `DELETE FROM rsvp` statement
     * despite `condeferred = t`, even though both tables end up empty by
     * COMMIT. See this entity's own migration
     * (Version20260811100000) docblock, point 3.
     */
    #[ORM\ManyToOne(targetEntity: Rsvp::class)]
    #[ORM\JoinColumn(name: 'related_rsvp_id', referencedColumnName: 'id', nullable: true, onDelete: 'NO ACTION')]
    private ?Rsvp $relatedRsvp = null;

    #[ORM\ManyToOne(targetEntity: Playlist::class)]
    #[ORM\JoinColumn(name: 'related_playlist_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Playlist $relatedPlaylist = null;

    /**
     * Deferred FK — `Forms`/Epic-08 does not exist in this codebase. Plain
     * nullable column, matching every other deferred-FK precedent in this
     * schema (`Rsvp::$paymentRecordId` before this same epic attached it).
     */
    #[ORM\Column(name: 'related_form_submission_id', type: 'bigint', nullable: true)]
    private ?int $relatedFormSubmissionId = null;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(
        Trainer $trainer,
        string $type,
        string $paymentMethod,
        int $amountMinorUnits,
        string $contactName,
        string $contactEmail,
        ?Account $payerAccount,
        ?string $contactPhone = null,
    ) {
        self::guardType($type);

        if (!\in_array($paymentMethod, [self::METHOD_TOKEN, self::METHOD_CARD], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown payment method "%s".', $paymentMethod));
        }

        if ($amountMinorUnits <= 0) {
            throw new \InvalidArgumentException('A payment amount must be greater than zero.');
        }

        if ('' === trim($contactName)) {
            throw new \InvalidArgumentException('A payment record requires a contact name.');
        }

        $this->trainer = $trainer;
        $this->type = $type;
        $this->paymentMethod = $paymentMethod;
        $this->amountMinorUnits = $amountMinorUnits;
        $this->contactName = $contactName;
        $this->contactEmail = $contactEmail;
        $this->contactPhone = $contactPhone;
        $this->payerAccount = $payerAccount;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    public static function guardType(string $type): void
    {
        $types = [self::TYPE_TOKEN_PURCHASE, self::TYPE_EVENT_RSVP, self::TYPE_CONTENT_PURCHASE, self::TYPE_PLAYER_SUBSCRIPTION, self::TYPE_CAMP_REGISTRATION];

        if (!\in_array($type, $types, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown payment record type "%s".', $type));
        }
    }

    /**
     * Builds the compensating record for a refund of $original — same
     * trainer/type/payer/contact/related-* linkage, `refundsPaymentRecord`
     * set, no fee rate snapshot (refund rows never carry one — the fee was
     * already assessed on the charge being refunded). `$amountMinorUnits`
     * is the amount actually refunded, which BR-05-5 allows to be a partial
     * token count but this codebase always refunds card payments in full
     * (BR-05-10: no partial-refund UI is built — see Q-05.05's resolution).
     */
    public static function forRefund(self $original, int $amountMinorUnits): self
    {
        if ($amountMinorUnits <= 0) {
            throw new \InvalidArgumentException('A refund amount must be greater than zero.');
        }

        $refund = new self(
            $original->trainer,
            $original->type,
            $original->paymentMethod,
            $amountMinorUnits,
            $original->contactName,
            $original->contactEmail,
            $original->payerAccount,
            $original->contactPhone,
        );
        $refund->refundsPaymentRecord = $original;
        $refund->relatedTokenPackage = $original->relatedTokenPackage;
        $refund->relatedRsvp = $original->relatedRsvp;
        $refund->relatedPlaylist = $original->relatedPlaylist;
        $refund->relatedFormSubmissionId = $original->relatedFormSubmissionId;
        $refund->status = self::STATUS_COMPLETED;

        return $refund;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getType(): string
    {
        return $this->type;
    }

    public function getPayerAccount(): ?Account
    {
        return $this->payerAccount;
    }

    public function getContactName(): string
    {
        return $this->contactName;
    }

    public function getContactEmail(): string
    {
        return $this->contactEmail;
    }

    public function getContactPhone(): ?string
    {
        return $this->contactPhone;
    }

    public function getPaymentMethod(): string
    {
        return $this->paymentMethod;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isPending(): bool
    {
        return self::STATUS_PENDING === $this->status;
    }

    public function isCompleted(): bool
    {
        return self::STATUS_COMPLETED === $this->status;
    }

    public function getAmountMinorUnits(): int
    {
        return $this->amountMinorUnits;
    }

    public function getFeeRateBasisPoints(): ?int
    {
        return $this->feeRateBasisPoints;
    }

    public function getPlatformFeeMinorUnits(): int
    {
        return $this->platformFeeMinorUnits;
    }

    public function getStripePaymentIntentId(): ?string
    {
        return $this->stripePaymentIntentId;
    }

    public function getStripeChargeId(): ?string
    {
        return $this->stripeChargeId;
    }

    public function getStripeRefundId(): ?string
    {
        return $this->stripeRefundId;
    }

    public function getRefundsPaymentRecord(): ?self
    {
        return $this->refundsPaymentRecord;
    }

    public function isRefund(): bool
    {
        return null !== $this->refundsPaymentRecord;
    }

    public function getRelatedTokenPackage(): ?TokenPackage
    {
        return $this->relatedTokenPackage;
    }

    public function getRelatedRsvp(): ?Rsvp
    {
        return $this->relatedRsvp;
    }

    public function getRelatedPlaylist(): ?Playlist
    {
        return $this->relatedPlaylist;
    }

    public function getRelatedFormSubmissionId(): ?int
    {
        return $this->relatedFormSubmissionId;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function attachRelatedTokenPackage(TokenPackage $tokenPackage): void
    {
        $this->relatedTokenPackage = $tokenPackage;
        $this->touch();
    }

    public function attachRelatedRsvp(Rsvp $rsvp): void
    {
        $this->relatedRsvp = $rsvp;
        $this->touch();
    }

    public function attachRelatedPlaylist(Playlist $playlist): void
    {
        $this->relatedPlaylist = $playlist;
        $this->touch();
    }

    /**
     * "Money and the platform fee — Where it runs": computed once, at
     * creation, from the rate in force at that moment, and never
     * recomputed. Never called on a refund row (constructed already
     * COMPLETED, with no fee of its own — see forRefund()).
     */
    public function applyFee(int $feeRateBasisPoints, int $platformFeeMinorUnits): void
    {
        if ($this->isRefund()) {
            throw new \LogicException('A refund record never carries its own fee snapshot.');
        }

        $this->feeRateBasisPoints = $feeRateBasisPoints;
        $this->platformFeeMinorUnits = $platformFeeMinorUnits;
        $this->touch();
    }

    /**
     * The Checkout Session's underlying PaymentIntent id, captured
     * synchronously at session-creation time (Stripe always creates one for
     * `mode=payment`) — this is what the webhook's `payment_intent.*`
     * events are looked up by (api-designer-spec.md "Stripe webhook
     * contract" step: "by looking up the PaymentRecord the PaymentIntent id
     * was stored against at Checkout-session creation").
     */
    public function attachStripePaymentIntentId(string $stripePaymentIntentId): void
    {
        $this->stripePaymentIntentId = $stripePaymentIntentId;
        $this->touch();
    }

    public function markCompleted(?string $stripeChargeId = null): void
    {
        $this->status = self::STATUS_COMPLETED;

        if (null !== $stripeChargeId) {
            $this->stripeChargeId = $stripeChargeId;
        }

        $this->touch();
    }

    public function markFailed(): void
    {
        $this->status = self::STATUS_FAILED;
        $this->touch();
    }

    /**
     * Applied to the ORIGINAL row once its compensating refund row (see
     * forRefund()) exists — this row's own status mirrors Stripe's
     * lifecycle, but "Refunded" as DISPLAYED is derived from the refund
     * row's existence (architect-architecture.md "Payment records").
     */
    public function markRefunded(?string $stripeRefundId): void
    {
        $this->status = self::STATUS_REFUNDED;

        if (null !== $stripeRefundId) {
            $this->stripeRefundId = $stripeRefundId;
        }

        $this->touch();
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
