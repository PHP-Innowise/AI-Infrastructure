<?php

declare(strict_types=1);

namespace App\Growth\Voter;

use App\Growth\Entity\Coupon;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Tenancy\TenantContext;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-06-8..11, AC-06-17..27: coupon management (trainer) and application
 * (player).
 *
 * `COUPON_CREATE` needs no object — "the query itself is scoped to the
 * current actor['s tenant]" (same shape as `TokenVoter::TOKEN_PURCHASE`),
 * since no `Coupon` exists yet at creation time.
 *
 * `COUPON_APPLY`'s own redeemability check (active, unexpired, under its
 * usage limit) is folded into this voter — the coarse "can this player even
 * attempt THIS coupon at all" gate. Applies-to-purchase-type and eligibility
 * (Q-06.10, still unresolved) depend on which SPECIFIC purchase is being
 * made, which this voter's `(actor, Coupon)` pair does not carry — those
 * stay in `CouponPricingService::quote()`, the seam the route table itself
 * calls "never the authoritative check" (re-run at final checkout
 * submission, not only at this voter's gate).
 *
 * No Super Admin clause on any attribute — no route or AC names a Super
 * Admin edit of an individual `Coupon`; the only Super-Admin-reachable
 * Growth screen is the platform-wide referral rule, a different voter
 * (`PlatformConfigurationVoter`) over a different (global) subject.
 *
 * @see specs/security-voter-designer-design.md "Growth module"
 */
/**
 * @extends Voter<string, Coupon|null>
 */
final class CouponVoter extends Voter
{
    public const COUPON_CREATE = 'COUPON_CREATE';
    public const COUPON_EDIT = 'COUPON_EDIT';
    public const COUPON_DEACTIVATE = 'COUPON_DEACTIVATE';
    public const COUPON_DELETE = 'COUPON_DELETE';
    public const COUPON_VIEW_ANALYTICS = 'COUPON_VIEW_ANALYTICS';
    public const COUPON_APPLY = 'COUPON_APPLY';

    private const TRAINER_ATTRIBUTES = [
        self::COUPON_EDIT,
        self::COUPON_DEACTIVATE,
        self::COUPON_DELETE,
        self::COUPON_VIEW_ANALYTICS,
    ];

    public function __construct(
        private readonly TenantContext $tenantContext,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::COUPON_CREATE => null === $subject,
            self::COUPON_APPLY => $subject instanceof Coupon,
            default => \in_array($attribute, self::TRAINER_ATTRIBUTES, true) && $subject instanceof Coupon,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::COUPON_CREATE === $attribute) {
            return AccountRole::Trainer === $actor->getRole();
        }

        if (self::COUPON_APPLY === $attribute) {
            \assert($subject instanceof Coupon);

            return AccountRole::Player === $actor->getRole()
                && $subject->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull()
                && $subject->isRedeemable(new \DateTimeImmutable());
        }

        \assert($subject instanceof Coupon);

        // AC-06-27: trainer-only, own tenant only — structurally same-
        // tenant already (RLS + the Doctrine filter make a foreign-tenant
        // coupon invisible before this voter runs), matching
        // TokenVoter::TOKEN_GIFT's own precedent.
        return AccountRole::Trainer === $actor->getRole()
            && $subject->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
    }
}
