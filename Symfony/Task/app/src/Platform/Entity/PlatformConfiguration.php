<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\PlatformConfigurationRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * Small, rarely-written, frequently-read singleton settings — a generic
 * key/JSONB-value row, not one column per setting. Global (no `TrainerScoped`
 * attribute, no RLS policy): this is platform-wide configuration, not
 * per-tenant data.
 *
 * Epic-06 is the first (and, in this codebase, only) writer: the referral
 * reward ratio (Q-06.11 default 1:1), the attribution window (Q-06.12
 * default 30 days), and the referee-welcome-token toggle (Q-06.11 default
 * off). `specs/database-designer-schema.md` also names
 * `default_platform_fee_basis_points`/`default_platform_subscription_price_minor_units`/
 * `token_package_seed` as eventual keys of this same table for Epic-05/07 —
 * not seeded here, since nothing in Epic-06 needs them and inventing values
 * for a different epic's settled behavior is out of this change's scope.
 *
 * @see specs/database-designer-schema.md "`platform_configuration` — Platform"
 * @see specs/architect-architecture.md "Entity population" — Global: "Referral ratio, default fee rate, default subscription price (BR-06-4)"
 */
#[ORM\Entity(repositoryClass: PlatformConfigurationRepository::class)]
#[ORM\Table(name: 'platform_configuration')]
#[ORM\UniqueConstraint(name: 'uniq_platform_configuration_key', columns: ['config_key'])]
class PlatformConfiguration
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\Column(name: 'config_key', type: 'string', length: 100)]
    private string $key;

    /**
     * @var mixed JSON-decoded on read, JSON-encoded on write by Doctrine's
     *            `json` type — never a raw string the caller must decode.
     */
    #[ORM\Column(type: 'json')]
    private mixed $value;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'updated_by_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $updatedByAccount = null;

    public function __construct(string $key, mixed $value, ?Account $updatedByAccount = null)
    {
        $this->key = $key;
        $this->value = $value;
        $this->updatedAt = new \DateTimeImmutable();
        $this->updatedByAccount = $updatedByAccount;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getKey(): string
    {
        return $this->key;
    }

    public function getValue(): mixed
    {
        return $this->value;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function getUpdatedByAccount(): ?Account
    {
        return $this->updatedByAccount;
    }

    public function replaceValue(mixed $value, ?Account $updatedByAccount): void
    {
        $this->value = $value;
        $this->updatedByAccount = $updatedByAccount;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
