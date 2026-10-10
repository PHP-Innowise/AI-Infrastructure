<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Billing\Repository\PaymentRecordRepository;
use App\Content\Billing\PaymentIntentGateway;
use App\Content\Billing\PaymentIntentOutcome;
use App\Content\Billing\PaymentIntentRequest;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAccessGrant;
use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Growth\Entity\Coupon;
use App\Growth\Exception\InvalidCouponException;
use App\Growth\Service\CouponPricingService;
use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Service\ChildActionAttempt;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\FundingMethod;
use App\Identity\Voter\ChildApprovalVoter;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\SecurityBundle\Security;

/**
 * US-04.07 (paywall)/BR-04-6..9, A8, A11: the one-time-purchase checkout
 * flow. Mirrors `App\Scheduling\Service\RsvpService`'s own paid-flow shape
 * closely — same child-approval bypass check before ever attempting
 * payment.
 *
 * Epic-05 (real Stripe/token gateway wired behind `PaymentIntentGateway`)
 * replaces the Epic-04 no-op: `attemptGrant()` now handles all three real
 * outcomes (Succeeded/Pending/Failed) via `PlaylistPurchaseOutcome`, since
 * a card payment needs a Checkout redirect URL a bare grant-or-null return
 * cannot carry.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-6..9, AC-04-22
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-18, AC-05-19
 */
final readonly class PurchasePlaylistAccessService
{
    public const METHOD_TOKEN = 'token';
    public const METHOD_USD = 'usd';

    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlaylistAccessGrantRepository $grants,
        private Security $security,
        private ChildApprovalService $childApprovalService,
        private PaymentIntentGateway $paymentGateway,
        private PlayerAccountResolver $playerAccounts,
        private PaymentRecordRepository $paymentRecords,
        private CouponPricingService $couponPricing,
    ) {
    }

    /**
     * @return list<string>
     */
    public static function paymentMethods(): array
    {
        return [self::METHOD_TOKEN, self::METHOD_USD];
    }

    /**
     * @throws InvalidCouponException
     */
    public function purchase(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod, ?string $couponCode = null): PlaylistPurchaseOutcome
    {
        $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

        if (null !== $existing) {
            return PlaylistPurchaseOutcome::granted($existing);
        }

        $originalAmount = $playlist->priceForMethod($paymentMethod);

        if ($originalAmount <= 0) {
            throw new \DomainException('This content has no price set for the chosen payment method.');
        }

        [$amount, $appliedCouponCode] = $this->applyCouponIfPresent($playlist, $player, $paymentMethod, $originalAmount, $couponCode);

        $fundingMethod = self::METHOD_USD === $paymentMethod ? FundingMethod::Usd : FundingMethod::Token;

        $bypassGranted = $this->security->isGranted(
            ChildApprovalVoter::CHILD_APPROVAL_BYPASS,
            new ChildActionAttempt($actor, $player, $fundingMethod),
        );

        if (!$bypassGranted) {
            $this->childApprovalService->requestApproval(
                $playlist->getTrainer(),
                $player,
                ChildApprovalRequest::ACTION_CONTENT_PURCHASE,
                requestedPlaylistId: (int) $playlist->getId(),
            );

            return PlaylistPurchaseOutcome::pendingApproval();
        }

        return $this->attemptGrant($playlist, $player, $actor, $paymentMethod, $amount, $appliedCouponCode);
    }

    /**
     * AC-06-20..23, BR-06-9: coupons only ever discount a CARD (usd)
     * purchase — the epic's own discount examples are exclusively USD
     * ("$20 event, 20% off = $16"), and a token price is an integer count
     * with no well-defined fractional-token discount. A token-funded
     * purchase silently ignores any submitted coupon code — a deliberate
     * scope decision recorded in the coder's final report, not a defect.
     *
     * @return array{0: int, 1: ?string} [amount to actually charge, the
     *                                    coupon code to carry through to
     *                                    checkout metadata (null if none
     *                                    applied)]
     *
     * @throws InvalidCouponException
     */
    private function applyCouponIfPresent(Playlist $playlist, PlayerProfile $player, string $paymentMethod, int $originalAmount, ?string $couponCode): array
    {
        if (self::METHOD_USD !== $paymentMethod || null === $couponCode || '' === trim($couponCode)) {
            return [$originalAmount, null];
        }

        $quote = $this->couponPricing->quote($playlist->getTrainer(), $couponCode, $player, Coupon::APPLIES_TO_CONTENT, $originalAmount);

        if (!$quote->valid) {
            // AC-06-23: "Invalid or expired code" error, no discount
            // applied — surfaced to the controller as a form error rather
            // than silently charging full price.
            throw InvalidCouponException::withMessage($quote->message);
        }

        \assert(null !== $quote->finalAmountMinorUnits && null !== $quote->coupon);

        return [$quote->finalAmountMinorUnits, $quote->coupon->getCode()];
    }

    /**
     * AC-01-26-style: the parent approved a pending ACTION_CONTENT_PURCHASE
     * request — proceeds exactly where purchase() would have gone had the
     * bypass been granted immediately. A child's original method choice is
     * not preserved through the approval window (see this method's own
     * historical note in git blame: `PlaylistAccessGrant` carries no such
     * column) — defaults to token, BR-04-6's primary-named option.
     */
    public function completeAfterParentApproval(Playlist $playlist, PlayerProfile $player, Account $payer): PlaylistPurchaseOutcome
    {
        $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

        if (null !== $existing) {
            return PlaylistPurchaseOutcome::granted($existing);
        }

        return $this->attemptGrant($playlist, $player, $payer, self::METHOD_TOKEN, $playlist->priceForMethod(self::METHOD_TOKEN));
    }

    private function attemptGrant(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod, int $amount, ?string $couponCode = null): PlaylistPurchaseOutcome
    {
        $payer = $this->playerAccounts->resolve($player) ?? $actor;
        $request = new PaymentIntentRequest($playlist->getTrainer(), $playlist, $player, $payer, $paymentMethod, $amount, $couponCode);
        $result = $this->paymentGateway->requestPayment($request);

        if (PaymentIntentOutcome::Failed === $result->outcome) {
            return PlaylistPurchaseOutcome::failed();
        }

        if (PaymentIntentOutcome::Pending === $result->outcome) {
            // AC-05-18: "Card: 303 to Stripe Checkout" — access stays
            // locked until the webhook confirms payment
            // (App\Content\EventSubscriber\PaymentOutcomeSubscriber).
            return PlaylistPurchaseOutcome::redirect((string) $result->redirectUrl);
        }

        \assert(null !== $result->paymentRecordId);

        return PlaylistPurchaseOutcome::granted($this->grantFor($playlist, $player, $payer, $result->paymentRecordId));
    }

    /**
     * Also the entry point `App\Content\EventSubscriber\PaymentOutcomeSubscriber`
     * calls once a card purchase's webhook confirms success — grant
     * issuance is identical either way, only how the payment resolved
     * differs.
     */
    public function grantFor(Playlist $playlist, PlayerProfile $player, Account $payer, int $paymentRecordId): PlaylistAccessGrant
    {
        return $this->entityManager->wrapInTransaction(function () use ($playlist, $player, $payer, $paymentRecordId): PlaylistAccessGrant {
            $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

            if (null !== $existing) {
                return $existing;
            }

            $paymentRecord = $this->paymentRecords->find($paymentRecordId)
                ?? throw new \LogicException(sprintf('Payment record %d not found while granting playlist access.', $paymentRecordId));

            $grant = new PlaylistAccessGrant($playlist->getTrainer(), $playlist, $player, $payer, $paymentRecord);
            $this->grants->add($grant);
            $this->entityManager->flush();

            return $grant;
        });
    }
}
