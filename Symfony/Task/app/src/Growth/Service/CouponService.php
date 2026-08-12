<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Entity\Coupon;
use App\Growth\Exception\CouponInUseException;
use App\Growth\Exception\DuplicateCouponCodeException;
use App\Growth\Repository\CouponRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-06.05/06.07: coupon creation, edit, deactivation, and deletion.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-17..19, AC-06-27, BR-06-8..11
 */
final readonly class CouponService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private CouponRepository $coupons,
    ) {
    }

    /**
     * AC-06-17/18: validated by `Coupon`'s own constructor; the trainer-
     * scoped uniqueness check is the one thing the entity cannot do for
     * itself (it would need a repository).
     *
     * @throws DuplicateCouponCodeException
     */
    public function create(
        Trainer $trainer,
        string $code,
        string $discountType,
        int $discountValue,
        string $appliesTo,
        ?int $usageLimit,
        string $eligibility,
        ?\DateTimeImmutable $expiresAt,
        bool $isActive,
        Account $createdBy,
    ): Coupon {
        if (null !== $this->coupons->findOneByTrainerAndCode($trainer, $code)) {
            throw DuplicateCouponCodeException::forCode($code);
        }

        $coupon = new Coupon($trainer, $code, $discountType, $discountValue, $appliesTo, $usageLimit, $eligibility, $expiresAt, $createdBy, $isActive);
        $this->coupons->add($coupon);
        $this->entityManager->flush();

        return $coupon;
    }

    /**
     * AC-06-27: edit is scoped to expiration, usage limit, and status —
     * code, discount type/value, applies-to, and eligibility are immutable
     * after creation (see `Coupon::updateEditableFields()`'s own docblock).
     */
    public function update(Coupon $coupon, ?\DateTimeImmutable $expiresAt, ?int $usageLimit, bool $isActive): void
    {
        $coupon->updateEditableFields($expiresAt, $usageLimit, $isActive);
        $this->entityManager->flush();
    }

    public function deactivate(Coupon $coupon): void
    {
        $coupon->deactivate();
        $this->entityManager->flush();
    }

    /**
     * AC-06-27: "delete it if it has never been used."
     *
     * @throws CouponInUseException
     */
    public function delete(Coupon $coupon): void
    {
        if ($coupon->hasBeenUsed()) {
            throw CouponInUseException::forCoupon($coupon);
        }

        $this->coupons->remove($coupon);
        $this->entityManager->flush();
    }
}
