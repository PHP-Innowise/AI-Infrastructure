<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\TokenBalanceRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * The locked projection over `TokenEntry` — I1 (balance = sum of entries),
 * I2 (never negative). One row per (trainer, parent account) pair, per
 * BR-05-2/BR-05-15: "a parent with multiple trainers holds a separate token
 * balance per trainer... tokens cannot be mixed or transferred between
 * trainers."
 *
 * No optimistic-locking `version` column: the fixed lock order ("the token
 * balance row first, then the event row" — architect-architecture.md "Lock
 * ordering") is pessimistic (`SELECT ... FOR UPDATE`) by design, not
 * optimistic. This entity is only ever mutated through
 * `TokenLedgerService`, inside a transaction that has already locked the
 * row via `TokenBalanceRepository::lockForUpdate()` — every call site goes
 * through that one method so the lock order cannot be gotten wrong.
 *
 * @see specs/database-designer-schema.md "`token_balance`"
 * @see specs/architect-architecture.md "The token and payment ledger — Lock ordering"
 */
#[ORM\Entity(repositoryClass: TokenBalanceRepository::class)]
#[ORM\Table(name: 'token_balance')]
#[ORM\UniqueConstraint(name: 'uniq_token_balance_trainer_parent', columns: ['trainer_id', 'parent_account_id'])]
#[TrainerScoped]
class TokenBalance
{
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

    #[ORM\Column(type: 'integer', options: ['default' => 0])]
    private int $balance = 0;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Trainer $trainer, Account $parentAccount)
    {
        $this->trainer = $trainer;
        $this->parentAccount = $parentAccount;
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

    public function getParentAccount(): Account
    {
        return $this->parentAccount;
    }

    public function getBalance(): int
    {
        return $this->balance;
    }

    /**
     * The ONLY mutator, and it is deliberately not "increment"/"decrement":
     * `TokenLedgerService` always computes the new total itself (current +
     * signed entry amount) and passes it here, so this method's own job is
     * only I2's application-layer half — the CHECK constraint is the
     * database-level backstop for the exact same rule. Only
     * `TokenLedgerService` calls this, always inside the same transaction
     * that appends the `TokenEntry` this projects, with this row already
     * locked via `TokenBalanceRepository::lockForUpdate()`.
     */
    public function applyNewTotal(int $newBalance): void
    {
        if ($newBalance < 0) {
            throw new \LogicException('I2: a token balance can never go negative.');
        }

        $this->balance = $newBalance;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
