<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\TokenEntryRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Entity\Event;
use Doctrine\ORM\Mapping as ORM;

/**
 * One append-only entry in the token ledger — the source of truth
 * `TokenBalance` is a locked projection over (architect-architecture.md
 * "The token and payment ledger").
 *
 * **I7 (append-only) is a database privilege, not a code convention.**
 * `pp_app` holds `INSERT, SELECT` only on this table (`REVOKE UPDATE,
 * DELETE` in Version20260811100000) — this class deliberately exposes NO
 * setters at all, only getters and the two named constructors below, so
 * even in-process there is no method that could mutate an already-persisted
 * row.
 *
 * Every invariant this table enforces (I1-I7) is indexed in
 * specs/database-designer-schema.md "The token and payment ledger" — this
 * class's own docblocks on each named constructor point at the specific
 * mechanism for the ones a CHECK constraint cannot express.
 *
 * @see specs/database-designer-schema.md "`token_entry`", "The token and payment ledger"
 * @see specs/architect-architecture.md "The token and payment ledger — Entry kinds and signs"
 */
#[ORM\Entity(repositoryClass: TokenEntryRepository::class)]
#[ORM\Table(name: 'token_entry')]
#[ORM\Index(name: 'idx_token_entry_trainer_parent_created', columns: ['trainer_id', 'parent_account_id', 'created_at'])]
#[ORM\Index(name: 'idx_token_entry_beneficiary', columns: ['beneficiary_player_id'])]
#[TrainerScoped]
class TokenEntry
{
    public const KIND_PURCHASE = 'purchase';
    public const KIND_GIFT = 'gift';
    public const KIND_REFERRAL_REWARD = 'referral_reward';
    public const KIND_REFUND = 'refund';
    public const KIND_SPEND = 'spend';
    public const KIND_ADJUSTMENT = 'adjustment';

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

    #[ORM\Column(type: 'string', length: 20)]
    private string $kind;

    /**
     * Signed. Positive for purchase/gift/referral_reward/refund, negative
     * for spend, either sign (never zero) for adjustment — I5.
     */
    #[ORM\Column(type: 'integer')]
    private int $amount;

    /**
     * Required for spend/refund (I3, A7 — "every token spend records the
     * beneficiary player, even though the balance sits at the
     * parent-trainer pair"). A `refund` entry's beneficiary is always
     * copied from the spend it references — see forRefund().
     */
    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'beneficiary_player_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?PlayerProfile $beneficiaryPlayer = null;

    /**
     * Required for `refund`, forbidden otherwise. Self-referencing — the
     * `spend` entry this refund compensates.
     */
    #[ORM\ManyToOne(targetEntity: self::class)]
    #[ORM\JoinColumn(name: 'refunds_entry_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?self $refundsEntry = null;

    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?PaymentRecord $paymentRecord = null;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'related_event_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Event $relatedEvent = null;

    /**
     * Deferred, unconstrained (no FK) — `ContentItem` lives in `Content`,
     * which Billing must not depend on for a write-time relation; a plain
     * id column matches the pattern every other cross-epic deferred column
     * in this schema uses.
     */
    #[ORM\Column(name: 'related_content_item_id', type: 'bigint', nullable: true)]
    private ?int $relatedContentItemId = null;

    /**
     * Deferred, unconstrained — `Referral`/Growth (Epic-06) does not exist
     * in this codebase.
     */
    #[ORM\Column(name: 'referral_id', type: 'bigint', nullable: true)]
    private ?int $referralId = null;

    /**
     * Set for gift/adjustment — who granted it (a trainer for `gift`, a
     * Super Admin for `adjustment` — BR-05-12/A7's own attribution rule).
     */
    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'performed_by_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $performedByAccount = null;

    #[ORM\Column(type: 'text')]
    private string $description;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    private function __construct(
        Trainer $trainer,
        Account $parentAccount,
        string $kind,
        int $amount,
        string $description,
        ?PlayerProfile $beneficiaryPlayer,
        ?self $refundsEntry,
        ?PaymentRecord $paymentRecord,
        ?Event $relatedEvent,
        ?int $relatedContentItemId,
        ?Account $performedByAccount,
    ) {
        if ('' === trim($description)) {
            throw new \InvalidArgumentException('A token entry requires a non-empty description.');
        }

        $this->trainer = $trainer;
        $this->parentAccount = $parentAccount;
        $this->kind = $kind;
        $this->amount = $amount;
        $this->description = $description;
        $this->beneficiaryPlayer = $beneficiaryPlayer;
        $this->refundsEntry = $refundsEntry;
        $this->paymentRecord = $paymentRecord;
        $this->relatedEvent = $relatedEvent;
        $this->relatedContentItemId = $relatedContentItemId;
        $this->performedByAccount = $performedByAccount;
        $this->createdAt = new \DateTimeImmutable();
    }

    /**
     * AC-05-4: a token purchase, funded by $paymentRecord.
     */
    public static function purchase(Trainer $trainer, Account $parentAccount, int $amount, PaymentRecord $paymentRecord, string $description): self
    {
        if ($amount <= 0) {
            throw new \InvalidArgumentException('A purchase entry amount must be positive.');
        }

        return new self($trainer, $parentAccount, self::KIND_PURCHASE, $amount, $description, null, null, $paymentRecord, null, null, null);
    }

    /**
     * AC-05-33: trainer-only, manual, no payment — BR-05-... "Trainer Can
     * Gift Tokens".
     */
    public static function gift(Trainer $trainer, Account $parentAccount, int $amount, Account $grantedBy, string $description): self
    {
        if ($amount <= 0) {
            throw new \InvalidArgumentException('A gift entry amount must be positive.');
        }

        return new self($trainer, $parentAccount, self::KIND_GIFT, $amount, $description, null, null, null, null, null, $grantedBy);
    }

    /**
     * Epic-06's own reward — kept here, not a separate `ReferralReward`
     * entity (architect-architecture.md Decisions, "Referral rewards"), so
     * Growth (once built) constructs this rather than inventing a second
     * record that could diverge from the balance. $referralId is a plain,
     * unconstrained int today — Growth's own migration attaches the FK.
     */
    public static function referralReward(Trainer $trainer, Account $parentAccount, int $amount, int $referralId, string $description): self
    {
        if ($amount <= 0) {
            throw new \InvalidArgumentException('A referral-reward entry amount must be positive.');
        }

        $entry = new self($trainer, $parentAccount, self::KIND_REFERRAL_REWARD, $amount, $description, null, null, null, null, null, null);
        $entry->referralId = $referralId;

        return $entry;
    }

    /**
     * AC-05-7/9: a token spend — RSVP or content purchase. A7: the
     * beneficiary player is always recorded, even though the balance sits
     * at the parent-trainer pair.
     */
    public static function spend(
        Trainer $trainer,
        Account $parentAccount,
        int $amount,
        PlayerProfile $beneficiaryPlayer,
        string $description,
        ?PaymentRecord $paymentRecord = null,
        ?Event $relatedEvent = null,
        ?int $relatedContentItemId = null,
    ): self {
        if ($amount <= 0) {
            throw new \InvalidArgumentException('A spend amount must be given as a positive count; the entry itself stores it negated.');
        }

        return new self($trainer, $parentAccount, self::KIND_SPEND, -$amount, $description, $beneficiaryPlayer, null, $paymentRecord, $relatedEvent, $relatedContentItemId, null);
    }

    /**
     * AC-05-14/16/17, BR-05-5: refunds tokens for a canceled RSVP/purchase.
     * I3's second half ("every refund inherits the beneficiary of the spend
     * it references") is enforced here, in code, not by a CHECK constraint
     * — Postgres CHECK constraints cannot read another row
     * (specs/database-designer-schema.md "`token_entry`"). $amount is
     * positive tokens returned; BR-05-5 permits a partial refund, so this
     * does not require $amount to equal the full original spend — the
     * caller (TokenLedgerService) is responsible for I4 (sum of refunds
     * against one spend never exceeds it), which needs a row lock and
     * cannot live in this entity either.
     */
    public static function refund(Trainer $trainer, self $spend, int $amount, string $description): self
    {
        if (self::KIND_SPEND !== $spend->kind) {
            throw new \InvalidArgumentException('A refund entry must reference a spend entry.');
        }

        if ($amount <= 0) {
            throw new \InvalidArgumentException('A refund amount must be positive.');
        }

        \assert(null !== $spend->beneficiaryPlayer, 'A spend entry always carries a beneficiary (chk_token_entry_beneficiary_required).');

        return new self(
            $trainer,
            $spend->parentAccount,
            self::KIND_REFUND,
            $amount,
            $description,
            $spend->beneficiaryPlayer,
            $spend,
            $spend->paymentRecord,
            $spend->relatedEvent,
            $spend->relatedContentItemId,
            null,
        );
    }

    /**
     * BR-05-12: Super-Admin-only out-of-band correction for Stripe
     * Dashboard activity the platform never saw a `payment_record` for.
     * Reason is required (folded into $description, which the CHECK
     * constraint already requires non-empty for every kind) and this is
     * always audit-logged by the caller (TokenLedgerService::adjust()).
     */
    public static function adjustment(Trainer $trainer, Account $parentAccount, int $amount, Account $superAdmin, string $reason): self
    {
        if (0 === $amount) {
            throw new \InvalidArgumentException('An adjustment amount cannot be zero.');
        }

        if ('' === trim($reason)) {
            throw new \InvalidArgumentException('BR-05-12: an adjustment requires a reason.');
        }

        return new self($trainer, $parentAccount, self::KIND_ADJUSTMENT, $amount, $reason, null, null, null, null, null, $superAdmin);
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

    public function getKind(): string
    {
        return $this->kind;
    }

    public function getAmount(): int
    {
        return $this->amount;
    }

    public function getBeneficiaryPlayer(): ?PlayerProfile
    {
        return $this->beneficiaryPlayer;
    }

    public function getRefundsEntry(): ?self
    {
        return $this->refundsEntry;
    }

    public function getPaymentRecord(): ?PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function getRelatedEvent(): ?Event
    {
        return $this->relatedEvent;
    }

    public function getRelatedContentItemId(): ?int
    {
        return $this->relatedContentItemId;
    }

    public function getReferralId(): ?int
    {
        return $this->referralId;
    }

    public function getPerformedByAccount(): ?Account
    {
        return $this->performedByAccount;
    }

    public function getDescription(): string
    {
        return $this->description;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }
}
