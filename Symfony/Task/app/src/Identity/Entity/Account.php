<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\AccountRepository;
use Doctrine\ORM\Mapping as ORM;
use Symfony\Component\Security\Core\User\PasswordAuthenticatedUserInterface;
use Symfony\Component\Security\Core\User\UserInterface;

/**
 * One row per person, regardless of how many trainers they are linked to.
 *
 * Global, not trainer-scoped: a player may work with several trainers and a
 * Super Admin belongs to none, so the account cannot carry a tenant key.
 * Which trainers an account can reach is `AccountTrainerLink`.
 *
 * @see specs/database-designer-schema.md "`account` — Platform/Identity"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-2, BR-01-7
 */
#[ORM\Entity(repositoryClass: AccountRepository::class)]
#[ORM\Table(name: 'account')]
#[ORM\Index(name: 'idx_account_role', columns: ['role'])]
#[ORM\Index(name: 'idx_account_status', columns: ['status'])]
class Account implements UserInterface, PasswordAuthenticatedUserInterface
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    /**
     * CITEXT, so uniqueness and lookup are case-insensitive without every
     * call site remembering to lowercase (BR-01-2).
     *
     * Typed as plain `string` to match the column: the non-emptiness is
     * guaranteed by the constructor and by a CHECK constraint, neither of
     * which the mapping type can express.
     */
    #[ORM\Column(type: 'string', length: 255, unique: true, columnDefinition: 'CITEXT NOT NULL')]
    private string $email;

    #[ORM\Column(type: 'string', length: 255)]
    private string $passwordHash;

    #[ORM\Column(type: 'string', length: 16, enumType: AccountRole::class)]
    private AccountRole $role;

    #[ORM\Column(type: 'string', length: 16, enumType: AccountStatus::class)]
    private AccountStatus $status = AccountStatus::Active;

    #[ORM\Column(type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $emailVerifiedAt = null;

    #[ORM\Column(type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $lastLoginAt = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    #[ORM\OneToOne(mappedBy: 'account', targetEntity: AccountProfile::class, cascade: ['persist'])]
    private ?AccountProfile $profile = null;

    public function __construct(string $email, string $passwordHash, AccountRole $role)
    {
        if ('' === trim($email)) {
            // The email is the security identifier. An empty one would make
            // getUserIdentifier() return a value the firewall cannot use.
            throw new \InvalidArgumentException('An account requires an email address.');
        }

        $this->email = $email;
        $this->passwordHash = $passwordHash;
        $this->role = $role;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getEmail(): string
    {
        return $this->email;
    }

    public function getUserIdentifier(): string
    {
        // Symfony documents this as non-empty-string. Emptiness is prevented
        // at construction and by chk_account_email_not_empty; this restates
        // the invariant where the type system can see it.
        \assert('' !== $this->email);

        return $this->email;
    }

    public function getPassword(): string
    {
        return $this->passwordHash;
    }

    public function getRole(): AccountRole
    {
        return $this->role;
    }

    /**
     * Exactly one role, never a list, and no hierarchy behind it. See
     * AccountRole for why the hierarchy is deliberately absent.
     *
     * @return list<string>
     */
    public function getRoles(): array
    {
        return [$this->role->securityRole()];
    }

    public function getStatus(): AccountStatus
    {
        return $this->status;
    }

    public function isActive(): bool
    {
        return $this->status->canLogIn();
    }

    public function isEmailVerified(): bool
    {
        return null !== $this->emailVerifiedAt;
    }

    public function verifyEmail(): void
    {
        $this->emailVerifiedAt ??= new \DateTimeImmutable();
        $this->touch();
    }

    /**
     * BR-01-23: blocks login; every historical record referencing this
     * account is untouched and stays visible.
     */
    public function deactivate(): void
    {
        if (AccountStatus::Deleted === $this->status) {
            throw new \LogicException('A deleted account cannot be deactivated — deletion is already permanent.');
        }

        $this->status = AccountStatus::Inactive;
        $this->touch();
    }

    /**
     * AC-01-54/58: reversible from Inactive; never from Deleted — GDPR
     * anonymisation is permanent, unlike deactivation.
     */
    public function reactivate(): void
    {
        if (AccountStatus::Deleted === $this->status) {
            throw new \LogicException('A deleted account cannot be reactivated — anonymisation is permanent.');
        }

        $this->status = AccountStatus::Active;
        $this->touch();
    }

    /**
     * AC-01-71: Super Admin edit form includes email. Uniqueness is the
     * caller's job (the DB-level CITEXT UNIQUE constraint is the final
     * backstop) — the entity only guards non-emptiness, same as the
     * constructor.
     */
    public function changeEmail(string $email): void
    {
        if ('' === trim($email)) {
            throw new \InvalidArgumentException('An account requires an email address.');
        }

        $this->email = $email;
        $this->touch();
    }

    /**
     * BR-01-24: the email becomes a deterministic, collision-free
     * placeholder. The caller supplies the value (`deleted_<id>@example.com`)
     * since the entity has no reason to know that format is the platform's
     * convention rather than its own.
     */
    public function anonymizeEmail(string $replacementEmail): void
    {
        $this->email = $replacementEmail;
        $this->touch();
    }

    /**
     * AC-01-56: status becomes Deleted. Permanent — see reactivate()'s own
     * guard.
     */
    public function markDeleted(): void
    {
        $this->status = AccountStatus::Deleted;
        $this->touch();
    }

    public function recordLogin(): void
    {
        $this->lastLoginAt = new \DateTimeImmutable();
    }

    /**
     * Takes an already-hashed value. The entity never sees, and never hashes,
     * a plaintext password — that belongs to the password hasher.
     */
    public function changePasswordHash(string $passwordHash): void
    {
        $this->passwordHash = $passwordHash;
        $this->touch();
    }

    public function getProfile(): ?AccountProfile
    {
        return $this->profile;
    }

    public function setProfile(AccountProfile $profile): void
    {
        $this->profile = $profile;
    }

    public function eraseCredentials(): void
    {
        // No plaintext credentials are ever held on this object.
    }

    private function touch(): void
    {
        $this->updatedAt = new \DateTimeImmutable();
    }
}
