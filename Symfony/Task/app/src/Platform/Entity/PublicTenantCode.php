<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Platform\Repository\PublicTenantCodeRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * Resolves an anonymous, code-bearing request to a trainer.
 *
 * This table exists to break a circularity: learning which trainer a ShareLink
 * belongs to requires reading `share_link`, which is itself trainer-scoped and
 * therefore unreadable until a trainer is already established. A tenancy-exempt
 * `code -> trainer` lookup is the precondition for resolving the tenant at all,
 * not one option among several.
 *
 * It holds nothing but the mapping. No expiry, no status, no use count — those
 * stay on the trainer-scoped ShareLink/Form, so this row can never drift into
 * an authority. A stale mapping grants a tenant and nothing else; the voter on
 * the underlying row is still what decides whether the actor may proceed.
 *
 * MUST NOT ever carry a Row-Level Security policy. See
 * config/tenancy/resolver_global_tables.txt.
 *
 * @see specs/council-sharelink-tenant-resolution.md
 */
#[ORM\Entity(repositoryClass: PublicTenantCodeRepository::class)]
#[ORM\Table(name: 'public_tenant_code')]
#[ORM\UniqueConstraint(name: 'uniq_public_tenant_code', columns: ['code'])]
#[ORM\Index(name: 'idx_ptc_reference', columns: ['kind', 'reference_id'])]
class PublicTenantCode
{
    public const KIND_SHARELINK = 'sharelink';
    public const KIND_FORM = 'form';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\Column(type: 'string', length: 64)]
    private string $code;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 16)]
    private string $kind;

    /**
     * Deliberately not a real foreign key: it points at `share_link` or `form`
     * depending on kind, and a column cannot reference two tables. Revocation
     * looks it up by (kind, reference_id).
     */
    #[ORM\Column(name: 'reference_id', type: 'bigint')]
    private int $referenceId;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(string $code, Trainer $trainer, string $kind, int $referenceId)
    {
        if (!\in_array($kind, [self::KIND_SHARELINK, self::KIND_FORM], true)) {
            throw new \InvalidArgumentException(sprintf('Unknown public tenant code kind "%s".', $kind));
        }

        $this->code = $code;
        $this->trainer = $trainer;
        $this->kind = $kind;
        $this->referenceId = $referenceId;
        $this->createdAt = new \DateTimeImmutable();
    }

    public function getCode(): string
    {
        return $this->code;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getKind(): string
    {
        return $this->kind;
    }

    public function getReferenceId(): int
    {
        return $this->referenceId;
    }
}
