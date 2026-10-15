<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\SubscriptionEntitlementRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * AC-05-29, BR-05-14: Player Subscriptions — "Unlimited Access", implemented
 * as an entitlement grant, never a Stripe Subscription object (BR-05-14:
 * "not a separate Stripe subscription"). Always `mode=payment`, one-time,
 * card-funded; the entitlement itself is granted only once
 * `ProcessStripeWebhookEventHandler` sees `payment_intent.succeeded` — "no
 * grace period."
 *
 * No `Billing` destination-charge routing here (unlike a card RSVP/content
 * purchase): a player subscription funds a PLATFORM entitlement, not a
 * trainer payout in the same shape a per-event/content sale is — but the
 * trainer still absorbed the 5% platform fee on the LISTED price the same
 * way (BR-05-7 is stated generally, not carved out for subscriptions), so
 * `StripeGateway::applyCurrentFee()` still runs; only the Connect
 * destination/transfer is skipped since this checkout is not itself a
 * per-trainer marketplace charge requiring a payout split at charge time —
 * the trainer's earnings still reconcile through their own Connect account
 * via Stripe's standard payout cycle once transferred, exactly as any other
 * charge on their account would. Recorded as a design note since no epic
 * spells out the Connect mechanics for this specific payment type.
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-29, BR-05-14
 */
final readonly class SubscriptionPurchaseService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PaymentRecordRepository $paymentRecords,
        private TrainerBillingSettingsRepository $billingSettings,
        private SubscriptionEntitlementRepository $entitlements,
        private StripeGateway $stripeGateway,
        private UrlGeneratorInterface $urlGenerator,
    ) {
    }

    /**
     * @throws \DomainException if the trainer has not enabled Player
     *                           Subscriptions (no price set), or if $payer already holds a
     *                           not-yet-expired entitlement for $trainer
     */
    public function purchase(Trainer $trainer, Account $payer, \DateTimeImmutable $activationDate): string
    {
        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
        $price = $settings->getPlayerSubscriptionPriceMinorUnits();

        if (null === $price) {
            throw new \DomainException('This trainer has not enabled Player Subscriptions.');
        }

        if ([] !== $this->entitlements->findNotYetExpired($trainer, $payer, new \DateTimeImmutable('today'))) {
            throw new \DomainException('An active or pending subscription already exists for this trainer.');
        }

        $paymentRecord = new PaymentRecord(
            $trainer,
            PaymentRecord::TYPE_PLAYER_SUBSCRIPTION,
            PaymentRecord::METHOD_CARD,
            $price,
            $this->contactNameFor($payer),
            $payer->getEmail(),
            $payer,
        );
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        $session = $this->stripeGateway->createCheckoutSession(
            $paymentRecord,
            $this->urlGenerator->generate('billing_portal_checkout_success', ['context' => 'subscription'], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_portal_checkout_cancel', ['context' => 'subscription'], UrlGeneratorInterface::ABSOLUTE_URL),
            metadata: ['activation_date' => $activationDate->format('Y-m-d')],
        );

        return $session->checkoutUrl;
    }

    private function contactNameFor(Account $account): string
    {
        $profile = $account->getProfile();

        return null !== $profile ? trim($profile->getFirstName().' '.$profile->getLastName()) : $account->getEmail();
    }
}
