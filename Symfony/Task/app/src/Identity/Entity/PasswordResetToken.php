<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\PasswordResetTokenRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * BR-01-4: expires 1 hour after issue. Doubles as the "set up your account"
 * link a newly-created trainer receives (AC-01-3, AC-01-4) — both are, at the
 * protocol level, "prove control of this email, then set a password", so no
 * separate token table is introduced for account setup.
 *
 * Only the SHA-256 hash is stored; the raw token exists solely in the emailed
 * link.
 *
 * @see specs/database-designer-schema.md "`password_reset_token`"
 */
#[ORM\Entity(repositoryClass: PasswordResetTokenRepository::class)]
#[ORM\Table(name: 'password_reset_token')]
#[ORM\UniqueConstraint(name: 'uniq_prt_token_hash', columns: ['token_hash'])]
#[ORM\Index(name: 'idx_prt_account', columns: ['account_id'])]
class PasswordResetToken
{
    private const TTL = 'PT1H';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'account_id', referencedColumnName: 'id', nullable: false, onDelete: 'CASCADE')]
    private Account $account;

    #[ORM\Column(name: 'token_hash', type: 'string', length: 255)]
    private string $tokenHash;

    #[ORM\Column(name: 'expires_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $expiresAt;

    #[ORM\Column(name: 'consumed_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $consumedAt = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Account $account, string $tokenHash)
    {
        $this->account = $account;
        $this->tokenHash = $tokenHash;
        $this->createdAt = new \DateTimeImmutable();
        $this->expiresAt = $this->createdAt->add(new \DateInterval(self::TTL));
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getAccount(): Account
    {
        return $this->account;
    }

    public function getTokenHash(): string
    {
        return $this->tokenHash;
    }

    public function getExpiresAt(): \DateTimeImmutable
    {
        return $this->expiresAt;
    }

    public function isConsumed(): bool
    {
        return null !== $this->consumedAt;
    }

    public function isUsable(\DateTimeImmutable $now): bool
    {
        return !$this->isConsumed() && $this->expiresAt > $now;
    }

    public function consume(): void
    {
        if ($this->isConsumed()) {
            throw new \LogicException('This password reset token has already been used.');
        }

        $this->consumedAt = new \DateTimeImmutable();
    }
}
