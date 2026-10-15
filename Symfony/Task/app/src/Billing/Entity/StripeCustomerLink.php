<?php

declare(strict_types=1);

namespace App\Billing\Entity;

use App\Billing\Repository\StripeCustomerLinkRepository;
use App\Identity\Entity\Account;
use Doctrine\ORM\Mapping as ORM;

/**
 * One Stripe Customer per account, shared across every trainer that account
 * trains with (BR-05-16, AC-05-21: "the same cards are shared across all of
 * a parent's trainers"). Global — an account is not itself trainer-scoped.
 *
 * `defaultPaymentMethodRef` is informational only: Stripe is the source of
 * truth (AC-05-21), read live via the Stripe Customer Portal, never
 * authoritative here.
 *
 * @see specs/database-designer-schema.md "`stripe_customer_link` — Billing"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md BR-05-16, AC-05-20, AC-05-21
 */
#[ORM\Entity(repositoryClass: StripeCustomerLinkRepository::class)]
#[ORM\Table(name: 'stripe_customer_link')]
#[ORM\UniqueConstraint(name: 'uniq_stripe_customer_link_account', columns: ['account_id'])]
class StripeCustomerLink
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $account;

    #[ORM\Column(name: 'stripe_customer_id', type: 'string', length: 255)]
    private string $stripeCustomerId;

    #[ORM\Column(name: 'default_payment_method_ref', type: 'string', length: 255, nullable: true)]
    private ?string $defaultPaymentMethodRef = null;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    public function __construct(Account $account, string $stripeCustomerId)
    {
        $this->account = $account;
        $this->stripeCustomerId = $stripeCustomerId;
        $this->createdAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getAccount(): Account
    {
        return $this->account;
    }

    public function getStripeCustomerId(): string
    {
        return $this->stripeCustomerId;
    }

    public function getDefaultPaymentMethodRef(): ?string
    {
        return $this->defaultPaymentMethodRef;
    }

    public function updateDefaultPaymentMethodRef(?string $ref): void
    {
        $this->defaultPaymentMethodRef = $ref;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }
}
