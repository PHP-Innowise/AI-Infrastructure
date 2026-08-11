<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Exception\InsufficientTokenBalanceException;
use App\Billing\Repository\PaymentRecordRepository;
use App\Content\Billing\PaymentIntentGateway;
use App\Content\Billing\PaymentIntentRequest;
use App\Content\Billing\PaymentIntentResult;
use App\Content\Service\PurchasePlaylistAccessService;
use App\Identity\Entity\Account;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * Epic-05's real `App\Content\Billing\PaymentIntentGateway`: Stripe
 * Checkout for `usd`, `TokenLedgerService` for `token`. Registered in
 * `config/services.yaml` in place of the removed no-op.
 *
 * Every `PaymentRecord` this class creates has `type = content_purchase`
 * and `relatedPlaylist` set.
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-18/19
 */
final readonly class ContentPaymentIntentGateway implements PaymentIntentGateway
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TokenLedgerService $tokenLedger,
        private StripeGateway $stripeGateway,
        private PaymentRecordRepository $paymentRecords,
        private UrlGeneratorInterface $urlGenerator,
    ) {
    }

    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult
    {
        return match ($request->paymentMethod) {
            PurchasePlaylistAccessService::METHOD_TOKEN => $this->spendTokens($request),
            PurchasePlaylistAccessService::METHOD_USD => $this->startCardCheckout($request),
            default => PaymentIntentResult::failed(),
        };
    }

    /**
     * AC-05-18: "Tokens (instant deduction)."
     */
    private function spendTokens(PaymentIntentRequest $request): PaymentIntentResult
    {
        $paymentRecord = $this->newPendingRecord($request, PaymentRecord::METHOD_TOKEN);
        $paymentRecord->attachRelatedPlaylist($request->playlist);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        try {
            $this->tokenLedger->spend(
                $request->trainer,
                $request->payer,
                $request->amount,
                $request->player,
                sprintf('Content purchase: %s', $request->playlist->getTitle()),
                $paymentRecord,
                relatedContentItemId: null,
            );
        } catch (InsufficientTokenBalanceException) {
            $paymentRecord->markFailed();
            $this->entityManager->flush();

            return PaymentIntentResult::failed();
        }

        $paymentRecord->markCompleted();
        $this->entityManager->flush();

        return PaymentIntentResult::succeeded((int) $paymentRecord->getId());
    }

    /**
     * AC-05-18: "Card (Stripe Checkout)."
     */
    private function startCardCheckout(PaymentIntentRequest $request): PaymentIntentResult
    {
        $paymentRecord = $this->newPendingRecord($request, PaymentRecord::METHOD_CARD);
        $paymentRecord->attachRelatedPlaylist($request->playlist);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        $session = $this->stripeGateway->createCheckoutSession(
            $paymentRecord,
            $this->urlGenerator->generate('billing_portal_checkout_success', ['context' => 'content'], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_portal_checkout_cancel', ['context' => 'content'], UrlGeneratorInterface::ABSOLUTE_URL),
            metadata: ['player_id' => (string) $request->player->getId()],
        );

        return PaymentIntentResult::pending((int) $paymentRecord->getId(), $session->checkoutUrl);
    }

    private function newPendingRecord(PaymentIntentRequest $request, string $paymentMethod): PaymentRecord
    {
        return new PaymentRecord(
            $request->trainer,
            PaymentRecord::TYPE_CONTENT_PURCHASE,
            $paymentMethod,
            $request->amount,
            $this->contactNameFor($request->payer),
            $request->payer->getEmail(),
            $request->payer,
        );
    }

    private function contactNameFor(Account $account): string
    {
        $profile = $account->getProfile();

        return null !== $profile ? trim($profile->getFirstName().' '.$profile->getLastName()) : $account->getEmail();
    }
}
