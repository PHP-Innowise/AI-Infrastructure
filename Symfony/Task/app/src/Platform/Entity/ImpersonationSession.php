<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\ImpersonationSessionRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * One row per impersonation session: who impersonated whom, when it started,
 * when (and why) it ended. AC-01-36's "Impersonation History" audit report is
 * a read over this table.
 *
 * BR-01-21 ("cannot target another Super Admin") is enforced by
 * `ImpersonationVoter`, not a CHECK here — the check would need to inspect
 * the target account's role, a cross-table fact a CHECK constraint cannot
 * see. The one same-row invariant this class *can* enforce — an admin cannot
 * "impersonate" themselves — is enforced in the constructor.
 *
 * The 1-hour expiry (BR-01-22) is derived at read time from `startedAt`,
 * never a scheduled flip of a status column.
 *
 * @see specs/database-designer-schema.md "`impersonation_session`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-21, BR-01-22, AC-01-33..38
 */
#[ORM\Entity(repositoryClass: ImpersonationSessionRepository::class)]
#[ORM\Table(name: 'impersonation_session')]
#[ORM\Index(name: 'idx_impersonation_target', columns: ['target_account_id'])]
#[ORM\Index(name: 'idx_impersonation_admin_open', columns: ['admin_account_id'])]
class ImpersonationSession
{
    public const REASON_MANUAL = 'manual';
    public const REASON_EXPIRED = 'expired';

    /**
     * BR-01-22.
     */
    public const DURATION_LIMIT = 'PT1H';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'admin_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $adminAccount;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'target_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $targetAccount;

    #[ORM\Column(name: 'started_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $startedAt;

    #[ORM\Column(name: 'ended_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $endedAt = null;

    #[ORM\Column(name: 'ended_reason', type: 'string', length: 16, nullable: true)]
    private ?string $endedReason = null;

    public function __construct(Account $adminAccount, Account $targetAccount)
    {
        if ($adminAccount === $targetAccount) {
            throw new \InvalidArgumentException('An account cannot impersonate itself.');
        }

        $this->adminAccount = $adminAccount;
        $this->targetAccount = $targetAccount;
        $this->startedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getAdminAccount(): Account
    {
        return $this->adminAccount;
    }

    public function getTargetAccount(): Account
    {
        return $this->targetAccount;
    }

    public function getStartedAt(): \DateTimeImmutable
    {
        return $this->startedAt;
    }

    public function getEndedAt(): ?\DateTimeImmutable
    {
        return $this->endedAt;
    }

    public function getEndedReason(): ?string
    {
        return $this->endedReason;
    }

    public function isOpen(): bool
    {
        return null === $this->endedAt;
    }

    /**
     * BR-01-22, derived at read time.
     */
    public function isExpired(\DateTimeImmutable $now): bool
    {
        return $this->isOpen() && $this->startedAt->add(new \DateInterval(self::DURATION_LIMIT)) < $now;
    }

    public function durationSeconds(): ?int
    {
        if (null === $this->endedAt) {
            return null;
        }

        return $this->endedAt->getTimestamp() - $this->startedAt->getTimestamp();
    }

    public function end(string $reason, ?\DateTimeImmutable $at = null): void
    {
        if (!\in_array($reason, [self::REASON_MANUAL, self::REASON_EXPIRED], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown impersonation end reason "%s".', $reason));
        }

        if (!$this->isOpen()) {
            throw new \LogicException('This impersonation session has already ended.');
        }

        $this->endedAt = $at ?? new \DateTimeImmutable();
        $this->endedReason = $reason;
    }
}
