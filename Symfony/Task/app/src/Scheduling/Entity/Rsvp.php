<?php

declare(strict_types=1);

namespace App\Scheduling\Entity;

use App\Billing\Entity\PaymentRecord;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Repository\RsvpRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * One player's registration for one event.
 *
 * `paymentRecord` was a deferred, nullable SCALAR FK before Epic-05 existed
 * (see git history — matching the pattern `ChildApprovalRequest` still uses
 * for its own two still-deferred columns). Epic-05's own migration
 * (Version20260811100000) attaches the real constraint, and this entity is
 * upgraded to a genuine `#[ORM\ManyToOne]` relation at the same time —
 * NOT kept as a scalar id the way the schema doc's own "deferred FK, plain
 * column" pattern would suggest, because Doctrine's fixture `ORMPurger`
 * (`doctrine:fixtures:load`, which `make test`/`make seed` both depend on)
 * computes table deletion order from ORM association metadata alone; a
 * scalar column carrying a real, unmapped database-level FK is invisible to
 * it, and purging failed outright once any fixture/test data actually
 * populated this column (`SQLSTATE[23503]`, "still referenced from table
 * rsvp") — discovered directly, not theoretical. `PlaylistAccessGrant`
 * already made this same choice for its own `payment_record_id` column;
 * this brings `Rsvp` in line with it, for the same reason.
 *
 * `status` intentionally excludes `attended`/`no_show` — attendance is its
 * own richer model, `AttendanceRecord` (BR-02-16/17) — see the schema's own
 * note under "`rsvp`".
 *
 * @see specs/database-designer-schema.md "`rsvp`"
 * @see specs/requirements-analyst-epic-02-event-management-spec.md BR-02-7..11, AC-02-23..33
 */
#[ORM\Entity(repositoryClass: RsvpRepository::class)]
#[ORM\Table(name: 'rsvp')]
#[ORM\UniqueConstraint(name: 'uniq_rsvp_event_player', columns: ['event_id', 'player_id'])]
#[ORM\Index(name: 'idx_rsvp_event_status', columns: ['event_id', 'status'])]
#[ORM\Index(name: 'idx_rsvp_player_status', columns: ['player_id', 'status'])]
#[TrainerScoped]
class Rsvp
{
    public const STATUS_PENDING_PARENT_APPROVAL = 'pending_parent_approval';
    public const STATUS_PENDING_PAYMENT = 'pending_payment';
    public const STATUS_CONFIRMED = 'confirmed';
    public const STATUS_CANCELED = 'canceled';

    public const METHOD_TOKEN = 'token';
    public const METHOD_USD = 'usd';
    public const METHOD_FREE = 'free';

    /**
     * Rows in these statuses hold a spot against capacity. A rejected or
     * never-completed payment (BR-02-9) never reaches here, so the capacity
     * count (AC-02-67) is exactly this set — see RsvpRepository::countHeld().
     */
    public const CAPACITY_HOLDING_STATUSES = [self::STATUS_PENDING_PARENT_APPROVAL, self::STATUS_PENDING_PAYMENT, self::STATUS_CONFIRMED];

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'event_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Event $event;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\Column(type: 'string', length: 24, options: ['default' => self::STATUS_PENDING_PAYMENT])]
    private string $status;

    #[ORM\Column(name: 'payment_method', type: 'string', length: 8)]
    private string $paymentMethod;

    /**
     * See the class docblock — a real ORM relation since Epic-05.
     *
     * `ON DELETE NO ACTION`, not `RESTRICT` — `payment_record.related_rsvp_id`
     * points back at this table, so the migration DDL for this constraint is
     * `DEFERRABLE INITIALLY DEFERRED` (hand-written; Doctrine's ORM
     * attributes cannot express DEFERRABLE, so this mapping is
     * documentation/schema-diff parity, not the source of truth). Postgres
     * silently ignores DEFERRABLE for the delete-triggered check when the
     * action is `RESTRICT` — only `NO ACTION` honors the deferred timing,
     * confirmed directly when `ORMPurger`'s single-transaction purge still
     * raised `SQLSTATE[23503]` synchronously under `RESTRICT` despite
     * `condeferred = t`. See `PaymentRecord::$relatedRsvp`'s own docblock
     * and Version20260811100000's docblock, point 3.
     */
    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: true, onDelete: 'NO ACTION')]
    private ?PaymentRecord $paymentRecord = null;

    #[ORM\Column(name: 'requested_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $requestedAt;

    #[ORM\Column(name: 'confirmed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $confirmedAt = null;

    #[ORM\Column(name: 'canceled_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $canceledAt = null;

    #[ORM\Column(name: 'cancellation_reason', type: 'text', nullable: true)]
    private ?string $cancellationReason = null;

    /**
     * Epic-05, deliberately NOT an ORM column (no `#[ORM\Column]`, so
     * Doctrine never persists or selects it): the one-time Stripe Checkout
     * URL a card RSVP's payment gateway call returns, carried from
     * `RsvpService::rsvp()` back to `PortalEventController` for the
     * same-request 303 redirect specs/api-designer-spec.md's "Billing
     * module" requires ("no separate 'create checkout session'
     * endpoint"). `PaymentRecord` itself has no such column (see its own
     * docblock) — a Checkout URL is used once, in memory, for the
     * duration of one request, never a durable fact about the RSVP.
     */
    private ?string $pendingCheckoutUrl = null;

    /**
     * BR-02-8: a free event RSVPs instantly, no payment step — the
     * constructor puts it straight to Confirmed. A paid event starts
     * Pending Payment (BR-02-9); the caller (RsvpService) moves it to
     * Pending Parent Approval instead, when a child's attempt is not
     * bypassed (BR-02-10), via markPendingParentApproval().
     */
    public function __construct(
        Trainer $trainer,
        Event $event,
        PlayerProfile $player,
        string $paymentMethod,
        \DateTimeImmutable $now,
    ) {
        if (!\in_array($paymentMethod, [self::METHOD_TOKEN, self::METHOD_USD, self::METHOD_FREE], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown RSVP payment method "%s".', $paymentMethod));
        }

        $this->trainer = $trainer;
        $this->event = $event;
        $this->player = $player;
        $this->paymentMethod = $paymentMethod;
        $this->requestedAt = $now;

        if (self::METHOD_FREE === $paymentMethod) {
            $this->status = self::STATUS_CONFIRMED;
            $this->confirmedAt = $now;
        } else {
            $this->status = self::STATUS_PENDING_PAYMENT;
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

    public function getEvent(): Event
    {
        return $this->event;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function getPaymentMethod(): string
    {
        return $this->paymentMethod;
    }

    public function getPaymentRecord(): ?PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function getRequestedAt(): \DateTimeImmutable
    {
        return $this->requestedAt;
    }

    public function getConfirmedAt(): ?\DateTimeImmutable
    {
        return $this->confirmedAt;
    }

    public function getCanceledAt(): ?\DateTimeImmutable
    {
        return $this->canceledAt;
    }

    public function getCancellationReason(): ?string
    {
        return $this->cancellationReason;
    }

    public function isConfirmed(): bool
    {
        return self::STATUS_CONFIRMED === $this->status;
    }

    public function isCanceled(): bool
    {
        return self::STATUS_CANCELED === $this->status;
    }

    public function isPaid(): bool
    {
        return self::METHOD_FREE !== $this->paymentMethod;
    }

    /**
     * BR-02-10: a child's attempt that is not bypassed waits here — "Pending
     * Parent Approval" — regardless of price (BR-02-10 applies to free RSVPs
     * too, per US-02.07/BR-02-10).
     *
     * Deliberately allowed FROM Confirmed too: the constructor (and
     * reactivate()) auto-confirm a free payment method unconditionally,
     * with no notion of a pending child-approval gate — RsvpService's own
     * caller here is the sole place that layers BR-02-10 on top, always
     * immediately after construction/reactivation and before anything else
     * observes the row (no flush, no notification has happened yet), so
     * "confirmed" at this exact point only ever means "free, a moment ago,
     * nothing has happened yet" — never a genuinely-settled confirmation
     * this would be wrong to override. An earlier version of this method
     * refused that transition outright, which made every free-event RSVP
     * attempt by a non-bypassed child a 500 instead of the pending state
     * BR-02-10 requires — found and fixed via RsvpTest's own
     * testChildRsvpRequiresParentApprovalRegardlessOfPrice.
     */
    public function markPendingParentApproval(): void
    {
        $this->status = self::STATUS_PENDING_PARENT_APPROVAL;
        $this->confirmedAt = null;
    }

    /**
     * AC-02-26: payment succeeded (or the parent's approval of a paid
     * request completed it) — confirms the RSVP.
     */
    public function confirm(\DateTimeImmutable $now): void
    {
        $this->status = self::STATUS_CONFIRMED;
        $this->confirmedAt = $now;
    }

    /**
     * Once a parent approves, a paid RSVP still needs its payment step —
     * this is the same "awaiting payment" state a direct adult attempt would
     * pass through, so the same confirm() call later completes it.
     */
    public function markPendingPayment(): void
    {
        $this->status = self::STATUS_PENDING_PAYMENT;
    }

    /**
     * AC-02-23..26: the parent approved a pending Pending Parent Approval
     * request — proceeds exactly where the original attempt would have gone
     * without the child gate (BR-02-8/9): confirmed immediately if free, or
     * into the payment step (Pending Payment) if paid. The caller
     * (RsvpService::completeAfterParentApproval()) runs the payment-intent
     * step next for the paid case, same as an ungated adult attempt would.
     */
    public function approveAfterParentApproval(\DateTimeImmutable $now): void
    {
        if (self::STATUS_PENDING_PARENT_APPROVAL !== $this->status) {
            throw new \LogicException('This RSVP is not awaiting parent approval.');
        }

        if (self::METHOD_FREE === $this->paymentMethod) {
            $this->status = self::STATUS_CONFIRMED;
            $this->confirmedAt = $now;
        } else {
            $this->status = self::STATUS_PENDING_PAYMENT;
        }
    }

    public function attachPaymentRecord(PaymentRecord $paymentRecord): void
    {
        $this->paymentRecord = $paymentRecord;
    }

    public function getPendingCheckoutUrl(): ?string
    {
        return $this->pendingCheckoutUrl;
    }

    public function setPendingCheckoutUrl(?string $url): void
    {
        $this->pendingCheckoutUrl = $url;
    }

    /**
     * AC-02-29/31, BR-02-11: cancellation. Refund eligibility/processing is
     * the caller's job (RsvpService, via the payment-intent gateway) —
     * this only records the cancellation fact.
     */
    public function cancel(?string $reason, \DateTimeImmutable $now): void
    {
        if (self::STATUS_CANCELED === $this->status) {
            throw new \LogicException('This RSVP is already canceled.');
        }

        $this->status = self::STATUS_CANCELED;
        $this->canceledAt = $now;
        $this->cancellationReason = $reason;
    }

    /**
     * BR-02-7's unique-per-(event, player) constraint carries no status
     * qualifier — reading the settled schema's own words, "one RSVP per
     * player per event" for the row's entire lifetime, not merely while
     * active. A player who canceled and later RSVPs again to the same event
     * therefore reuses this same row rather than inserting a second one
     * (which the database would reject outright) — the same
     * reactivate-the-existing-row pattern
     * `PlayerTrainerMembership::reactivate()` and
     * `CoachAssignment::reassign()` already establish elsewhere in this
     * codebase for exactly this shape of constraint.
     */
    public function reactivate(string $paymentMethod, \DateTimeImmutable $now): void
    {
        if (!\in_array($paymentMethod, [self::METHOD_TOKEN, self::METHOD_USD, self::METHOD_FREE], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown RSVP payment method "%s".', $paymentMethod));
        }

        $this->paymentMethod = $paymentMethod;
        $this->paymentRecord = null;
        $this->requestedAt = $now;
        $this->canceledAt = null;
        $this->cancellationReason = null;

        if (self::METHOD_FREE === $paymentMethod) {
            $this->status = self::STATUS_CONFIRMED;
            $this->confirmedAt = $now;
        } else {
            $this->status = self::STATUS_PENDING_PAYMENT;
            $this->confirmedAt = null;
        }
    }
}
