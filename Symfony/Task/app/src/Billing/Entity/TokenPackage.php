<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\TokenPackageRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A trainer-configured token bundle (BR-05-1) — "10 tokens/$90", etc.
 *
 * A custom-amount purchase (AC-05-4's "or a custom amount") is modelled as
 * its OWN `TokenPackage` row with `isActive = false`: `payment_record`'s own
 * CHECK constraint requires every `token_purchase` row to reference exactly
 * one package (matching every other `payment_record` type's "exactly one
 * related-thing" shape — see Version20260811100000's own docblock), so a
 * one-off custom amount gets a one-off, unlisted package rather than a
 * second, divergent code path. `isActive = false` keeps it out of
 * `billing_trainer_pricing_edit`'s trainer-visible list and out of
 * `billing_portal_tokens_purchase`'s package picker — it exists solely to
 * be the one thing this specific payment record points at.
 *
 * @see specs/database-designer-schema.md "`token_package`"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md BR-05-1, AC-05-4, Q-05.03
 */
#[ORM\Entity(repositoryClass: TokenPackageRepository::class)]
#[ORM\Table(name: 'token_package')]
#[ORM\Index(name: 'idx_token_package_trainer_active', columns: ['trainer_id', 'is_active'])]
#[TrainerScoped]
class TokenPackage
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(type: 'string', length: 100)]
    private string $label;

    #[ORM\Column(name: 'token_count', type: 'integer')]
    private int $tokenCount;

    #[ORM\Column(name: 'price_minor_units', type: 'integer')]
    private int $priceMinorUnits;

    #[ORM\Column(name: 'is_active', type: 'boolean', options: ['default' => true])]
    private bool $isActive;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Trainer $trainer, string $label, int $tokenCount, int $priceMinorUnits, bool $isActive = true)
    {
        if ($tokenCount <= 0) {
            throw new \InvalidArgumentException('A token package must contain at least one token.');
        }

        if ($priceMinorUnits <= 0) {
            throw new \InvalidArgumentException('A token package price must be greater than zero.');
        }

        $this->trainer = $trainer;
        $this->label = $label;
        $this->tokenCount = $tokenCount;
        $this->priceMinorUnits = $priceMinorUnits;
        $this->isActive = $isActive;
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

    public function getLabel(): string
    {
        return $this->label;
    }

    public function getTokenCount(): int
    {
        return $this->tokenCount;
    }

    public function getPriceMinorUnits(): int
    {
        return $this->priceMinorUnits;
    }

    public function isActive(): bool
    {
        return $this->isActive;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    /**
     * `billing_trainer_pricing_edit`: the trainer edits a listed package's
     * label/count/price, or retires it (AC-05-1..2, BR-05-1). Never called
     * on a custom-amount package (see this class's own docblock).
     */
    public function update(string $label, int $tokenCount, int $priceMinorUnits, bool $isActive): void
    {
        if ($tokenCount <= 0) {
            throw new \InvalidArgumentException('A token package must contain at least one token.');
        }

        if ($priceMinorUnits <= 0) {
            throw new \InvalidArgumentException('A token package price must be greater than zero.');
        }

        $this->label = $label;
        $this->tokenCount = $tokenCount;
        $this->priceMinorUnits = $priceMinorUnits;
        $this->isActive = $isActive;
    }
}
