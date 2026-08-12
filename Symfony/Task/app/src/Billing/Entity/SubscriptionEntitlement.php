<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\SubscriptionEntitlementRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * "The subscription token" — BR-05-14's Player Subscription, an entitlement
 * GRANT with its own window and state, never a ledger entry
 * (architect-architecture.md Decisions, "Subscriptions": "its expiry
 * contradicts BR-05-4's no-expiry rule for tokens").
 *
 * `windowEndsOn` is a PostgreSQL `GENERATED ALWAYS` column
 * (`activation_date + 30`), so it is never set from PHP — read back after
 * insert like any other database default.
 *
 * **No stored `status` column.** `pending_activation` / `active` / `expired`
 * are derived at read time from `activationDate`/`windowEndsOn` against
 * "today" in the trainer's own timezone (architect-architecture.md, Open
 * architecture risk #1) — see `statusOn()`.
 *
 * The database's own `EXCLUDE USING gist` constraint (not expressible here)
 * is the actual enforcement that no two entitlement windows for the same
 * (trainer, parent) pair may overlap — this class does not re-check it, the
 * same way no entity in this codebase re-checks a unique index.
 *
 * @see specs/database-designer-schema.md "`subscription_entitlement`"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-29, BR-05-14
 */
#[ORM\Entity(repositoryClass: SubscriptionEntitlementRepository::class)]
#[ORM\Table(name: 'subscription_entitlement')]
#[ORM\Index(name: 'idx_subscription_entitlement_lookup', columns: ['trainer_id', 'parent_account_id', 'activation_date', 'window_ends_on'])]
#[TrainerScoped]
class SubscriptionEntitlement
{
    public const STATUS_PENDING_ACTIVATION = 'pending_activation';
    public const STATUS_ACTIVE = 'active';
    public const STATUS_EXPIRED = 'expired';

    /**
     * BR-05-14: a fixed 30-day window from the chosen activation date.
     */
    private const WINDOW_DAYS = 30;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'parent_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $parentAccount;

    #[ORM\Column(name: 'activation_date', type: 'date_immutable')]
    private \DateTimeImmutable $activationDate;

    /**
     * `GENERATED ALWAYS AS (activation_date + 30) STORED` — never written
     * from PHP. `insertable: false, updatable: false` tells Doctrine to
     * omit it from INSERT/UPDATE and simply read it back.
     */
    #[ORM\Column(name: 'window_ends_on', type: 'date_immutable', insertable: false, updatable: false)]
    private \DateTimeImmutable $windowEndsOn;

    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: false, unique: true, onDelete: 'RESTRICT')]
    private PaymentRecord $paymentRecord;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    /**
     * BR-05-14: "the trainer can activate subscription from any future
     * date" — $activationDate may be today or any future date, never past
     * (a subscription purchased today cannot retroactively have covered
     * yesterday). Granted only by the webhook-confirmed payment handler
     * (BR-05-14's "no grace period" — see
     * App\Billing\MessageHandler\ProcessStripeWebhookEventHandler), never
     * at purchase-request time.
     */
    public function __construct(Trainer $trainer, Account $parentAccount, \DateTimeImmutable $activationDate, PaymentRecord $paymentRecord)
    {
        $this->trainer = $trainer;
        $this->parentAccount = $parentAccount;
        $this->activationDate = $activationDate;
        // Computed here too, in PHP, purely so a freshly-constructed
        // (not-yet-flushed) instance already answers statusOn() correctly
        // in the same request that creates it — overwritten by the
        // database's own GENERATED value on the next read regardless.
        $this->windowEndsOn = $activationDate->modify(sprintf('+%d days', self::WINDOW_DAYS));
        $this->paymentRecord = $paymentRecord;
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

    public function getParentAccount(): Account
    {
        return $this->parentAccount;
    }

    public function getActivationDate(): \DateTimeImmutable
    {
        return $this->activationDate;
    }

    public function getWindowEndsOn(): \DateTimeImmutable
    {
        return $this->windowEndsOn;
    }

    public function getPaymentRecord(): PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    /**
     * Derived at read time — architecture "Time-based state is derived at
     * read time". $today is the trainer-timezone calendar date (Open
     * architecture risk #1).
     */
    public function statusOn(\DateTimeImmutable $today): string
    {
        if ($today < $this->activationDate) {
            return self::STATUS_PENDING_ACTIVATION;
        }

        if ($today > $this->windowEndsOn) {
            return self::STATUS_EXPIRED;
        }

        return self::STATUS_ACTIVE;
    }

    public function isActiveOn(\DateTimeImmutable $today): bool
    {
        return self::STATUS_ACTIVE === $this->statusOn($today);
    }
}
