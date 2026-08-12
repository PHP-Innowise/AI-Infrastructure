<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\TokenEntry;
use App\Billing\Exception\InsufficientTokenBalanceException;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\TokenEntryRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Scheduling\Billing\FundingShortfall;
use App\Scheduling\Billing\PaymentIntentGateway;
use App\Scheduling\Billing\PaymentIntentRequest;
use App\Scheduling\Billing\PaymentIntentResult;
use App\Scheduling\Entity\Rsvp;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * Epic-05's real `App\Scheduling\Billing\PaymentIntentGateway`: Stripe
 * Checkout for `usd`, `TokenLedgerService` for `token` — exactly what that
 * interface's own docblock, and `NoopPaymentIntentGateway`'s docblock
 * before it, said would eventually implement it. Registered in
 * `config/services.yaml` in place of the removed no-op.
 *
 * Every `PaymentRecord` this class creates has `type = event_rsvp` and
 * `relatedRsvp` set — AC-05-22/23's unified transaction history reads these
 * rows regardless of `paymentMethod`, so a token-funded RSVP appears there
 * exactly like a card one, not only USD purchases.
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-7..17
 */
final readonly class SchedulingPaymentIntentGateway implements PaymentIntentGateway
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TokenLedgerService $tokenLedger,
        private StripeGateway $stripeGateway,
        private PaymentRecordRepository $paymentRecords,
        private TokenEntryRepository $tokenEntries,
        private UrlGeneratorInterface $urlGenerator,
    ) {
    }

    public function lockFundingForUpdate(Trainer $trainer, Account $payer, string $paymentMethod): void
    {
        if (Rsvp::METHOD_TOKEN === $paymentMethod) {
            $this->tokenLedger->lockBalanceForUpdate($trainer, $payer);
        }
    }

    /**
     * Only a token payment can be short: a card is funded at Stripe
     * Checkout, and a free RSVP has nothing to fund.
     */
    public function findFundingShortfall(Trainer $trainer, Account $payer, string $paymentMethod, int $amount): ?FundingShortfall
    {
        if (Rsvp::METHOD_TOKEN !== $paymentMethod) {
            return null;
        }

        $available = $this->tokenLedger->balanceFor($trainer, $payer);

        return $available < $amount ? new FundingShortfall($available, $amount) : null;
    }

    public function requestPayment(PaymentIntentRequest $request): PaymentIntentResult
    {
        return match ($request->paymentMethod) {
            Rsvp::METHOD_TOKEN => $this->spendTokens($request),
            Rsvp::METHOD_USD => $this->startCardCheckout($request),
            default => PaymentIntentResult::failed(),
        };
    }

    public function requestRefund(PaymentIntentRequest $request): PaymentIntentResult
    {
        $original = $this->paymentRecords->findOneChargeByRsvp($request->rsvp);

        if (null === $original || !$original->isCompleted()) {
            return PaymentIntentResult::failed();
        }

        if (PaymentRecord::METHOD_TOKEN === $original->getPaymentMethod()) {
            return $this->refundTokens($request, $original);
        }

        $refundRecord = $this->stripeGateway->refund($original, $original->getAmountMinorUnits());

        return null !== $refundRecord
            ? PaymentIntentResult::succeeded((int) $refundRecord->getId())
            : PaymentIntentResult::failed();
    }

    /**
     * AC-05-7/9: instant, no payment-processing delay — the whole thing
     * (payment record creation, the ledger spend, marking the record
     * completed) runs inside the caller's already-open transaction
     * (`RsvpService::rsvp()`), with the token balance row already locked by
     * `lockFundingForUpdate()` before the event row.
     */
    private function spendTokens(PaymentIntentRequest $request): PaymentIntentResult
    {
        $paymentRecord = $this->newPendingRecord($request, PaymentRecord::METHOD_TOKEN);
        $paymentRecord->attachRelatedRsvp($request->rsvp);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        try {
            $this->tokenLedger->spend(
                $request->trainer,
                $request->payer,
                $request->amount,
                $request->rsvp->getPlayer(),
                sprintf('Event RSVP: %s', $request->rsvp->getEvent()->getTitle()),
                $paymentRecord,
                $request->rsvp->getEvent(),
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
     * AC-05-10..13: creates the Checkout Session and returns its URL for
     * the caller (`PortalEventController`) to redirect to in the SAME
     * request — specs/api-designer-spec.md "Billing module": "every
     * purchase-initiating route... performs the 303 redirect... in the same
     * request; there is no separate 'create checkout session' endpoint."
     */
    private function startCardCheckout(PaymentIntentRequest $request): PaymentIntentResult
    {
        $paymentRecord = $this->newPendingRecord($request, PaymentRecord::METHOD_CARD);
        $paymentRecord->attachRelatedRsvp($request->rsvp);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        // Epic-06: 'coupon_code' travels via Stripe Checkout metadata,
        // echoed back verbatim on PaymentRecordSettled — see
        // App\Content\Billing\PaymentIntentRequest::$couponCode's own
        // docblock for the identical mechanism ContentPaymentIntentGateway
        // already uses.
        $metadata = null !== $request->couponCode ? ['coupon_code' => $request->couponCode] : [];

        $session = $this->stripeGateway->createCheckoutSession(
            $paymentRecord,
            $this->urlGenerator->generate('billing_portal_checkout_success', ['context' => 'rsvp'], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_portal_checkout_cancel', ['context' => 'rsvp'], UrlGeneratorInterface::ABSOLUTE_URL),
            metadata: $metadata,
        );

        return PaymentIntentResult::pending((int) $paymentRecord->getId(), $session->checkoutUrl);
    }

    /**
     * AC-05-14/17: instant token refund.
     */
    private function refundTokens(PaymentIntentRequest $request, PaymentRecord $original): PaymentIntentResult
    {
        $spendEntry = $this->tokenEntries->findOneByPaymentRecordAndKind($original, TokenEntry::KIND_SPEND);

        if (null === $spendEntry) {
            return PaymentIntentResult::failed();
        }

        $refundEntry = $this->tokenLedger->refund(
            $request->trainer,
            $spendEntry,
            $request->amount,
            sprintf('Refund: Event RSVP %s', $request->rsvp->getEvent()->getTitle()),
        );

        $refundRecord = PaymentRecord::forRefund($original, $request->amount);
        $this->paymentRecords->add($refundRecord);
        $original->markRefunded(null);
        $this->entityManager->flush();

        \assert(null !== $refundEntry->getId());

        return PaymentIntentResult::succeeded((int) $refundRecord->getId());
    }

    private function newPendingRecord(PaymentIntentRequest $request, string $paymentMethod): PaymentRecord
    {
        return new PaymentRecord(
            $request->trainer,
            PaymentRecord::TYPE_EVENT_RSVP,
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
