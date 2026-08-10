<?php

declare(strict_types=1);

namespace App\Content\Service;

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
 * payment, same "the shipped Noop gateway always stalls at Pending, tests
 * prove the Succeeded branch with a stub" honesty.
 *
 * `paymentMethod` is a request-time-only value: `PlaylistAccessGrant` has no
 * column for it (see that entity's own docblock — the schema stores WHAT
 * was bought, not how). A child's attempt that is queued for parent
 * approval therefore cannot carry the originally-chosen method through the
 * approval window the way `Rsvp::paymentMethod` (a real column) does for
 * RSVPs; `completeAfterParentApproval()` defaults to `token`
 * (BR-04-6's primary-named option) — documented here rather than silently
 * assumed, and immaterial to today's actual behavior since
 * `NoopPaymentIntentGateway` ignores the method entirely regardless.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-6..9, AC-04-22
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
     * Returns the grant once access is unlocked, or null when the attempt
     * is still pending (parent approval queued, or the payment gateway has
     * not yet confirmed — always true today, see the class docblock).
     * AC-05-18-style idempotency: an already-purchased playlist returns its
     * existing grant immediately, without a second gateway call.
     */
    public function purchase(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod): ?PlaylistAccessGrant
    {
        $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

        if (null !== $existing) {
            return $existing;
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

            return null;
        }

        return $this->attemptGrant($playlist, $player, $actor, $paymentMethod);
    }

    /**
     * AC-01-26-style: the parent approved a pending ACTION_CONTENT_PURCHASE
     * request — proceeds exactly where purchase() would have gone had the
     * bypass been granted immediately. Called by the Content-owned
     * post-decision route this module's own `ApprovalController` branch
     * redirects to (Identity must never call into Content directly).
     */
    public function completeAfterParentApproval(Playlist $playlist, PlayerProfile $player, Account $payer): ?PlaylistAccessGrant
    {
        $existing = $this->grants->findOneByPlaylistAndPlayer($playlist, $player);

        if (null !== $existing) {
            return $existing;
        }

        return $this->attemptGrant($playlist, $player, $payer, self::METHOD_TOKEN);
    }

    private function attemptGrant(Playlist $playlist, PlayerProfile $player, Account $actor, string $paymentMethod): ?PlaylistAccessGrant
    {
        $payer = $this->playerAccounts->resolve($player) ?? $actor;
        // BR-04-8: no real price is modeled yet (D-SCOPE-011 is unresolved
        // in the source, and specs/database-designer-schema.md stores no
        // price column on `playlist`) — see PaymentIntentRequest's own
        // docblock for why `0` is a structural placeholder, not a claim
        // about actual cost.
        $request = new PaymentIntentRequest($playlist->getTrainer(), $playlist, $player, $payer, $paymentMethod, 0);
        $outcome = $this->paymentGateway->requestPayment($request);

        if (PaymentIntentOutcome::Succeeded !== $outcome) {
            // AC-04-22: "opens the purchase/payment flow" — with the shipped
            // NoopPaymentIntentGateway this always stays locked/pending,
            // which is the honest, correct state until Epic-05 exists (see
            // NoopPaymentIntentGateway's own docblock). The Succeeded branch
            // below is still fully exercised by tests supplying a stub
            // gateway.
            return null;
        }

        return $this->entityManager->wrapInTransaction(function () use ($playlist, $player, $payer): PlaylistAccessGrant {
            $grant = new PlaylistAccessGrant($playlist->getTrainer(), $playlist, $player, $payer);
            $this->grants->add($grant);
            $this->entityManager->flush();

            return $grant;
        });
    }
}
