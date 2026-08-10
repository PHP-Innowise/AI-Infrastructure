<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\EmailVerificationTokenRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * BR-01-5: expires 24 hours after issue. Only the SHA-256 hash is stored —
 * the raw token exists solely in the emailed link, matching PasswordResetToken.
 *
 * @see specs/database-designer-schema.md "`email_verification_token`"
 */
#[ORM\Entity(repositoryClass: EmailVerificationTokenRepository::class)]
#[ORM\Table(name: 'email_verification_token')]
#[ORM\UniqueConstraint(name: 'uniq_evt_token_hash', columns: ['token_hash'])]
#[ORM\Index(name: 'idx_evt_account', columns: ['account_id'])]
class EmailVerificationToken
{
    private const TTL = 'PT24H';

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
            throw new \LogicException('This email verification token has already been used.');
        }

        $this->consumedAt = new \DateTimeImmutable();
    }
}
