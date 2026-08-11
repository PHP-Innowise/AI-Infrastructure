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
    ) {
    }

    /**
     * @return list<string>
     */
    public static function paymentMethods(): array
    {
        return [self::METHOD_TOKEN, self::METHOD_USD];
    }

    public function purchase(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod): PlaylistPurchaseOutcome
    {
        $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

        if (null !== $existing) {
            return PlaylistPurchaseOutcome::granted($existing);
        }

        if ($playlist->priceForMethod($paymentMethod) <= 0) {
            throw new \DomainException('This content has no price set for the chosen payment method.');
        }

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

        return $this->attemptGrant($playlist, $player, $actor, $paymentMethod);
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

        return $this->attemptGrant($playlist, $player, $payer, self::METHOD_TOKEN);
    }

    private function attemptGrant(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod): PlaylistPurchaseOutcome
    {
        $payer = $this->playerAccounts->resolve($player) ?? $actor;
        $amount = $playlist->priceForMethod($paymentMethod);
        $request = new PaymentIntentRequest($playlist->getTrainer(), $playlist, $player, $payer, $paymentMethod, $amount);
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
