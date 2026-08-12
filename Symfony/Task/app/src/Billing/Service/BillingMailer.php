<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;

/**
 * Every Epic-05 transactional email, matching
 * `App\Scheduling\Service\SchedulingMailer`'s own precedent — plain-text
 * `Email`, not `TemplatedEmail` + Twig.
 */
final readonly class BillingMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
    ) {
    }

    /**
     * AC-05-4: confirmation email + receipt once a token purchase
     * completes.
     */
    public function sendTokenPurchaseConfirmed(PaymentRecord $paymentRecord, int $tokenCount): void
    {
        $email = (new Email())
            ->from(self::FROM)
            ->to($paymentRecord->getContactEmail())
            ->subject('Token purchase confirmed')
            ->text(sprintf("Your purchase of %d tokens is confirmed.\n\nThank you!", $tokenCount));

        $this->mailer->send($email);
    }

    /**
     * Technical Notes § "Error Handling": "payment failures show clear,
     * user-friendly messages" — the email half of that, for an async
     * (Stripe-Checkout) payment that ultimately failed.
     */
    public function sendPaymentFailed(PaymentRecord $paymentRecord): void
    {
        $email = (new Email())
            ->from(self::FROM)
            ->to($paymentRecord->getContactEmail())
            ->subject('Payment could not be completed')
            ->text("Your recent payment attempt was not successful. Please try again or use a different payment method.");

        $this->mailer->send($email);
    }

    /**
     * AC-05-29: subscription confirmation once the webhook grants the
     * entitlement.
     */
    public function sendSubscriptionActivated(PaymentRecord $paymentRecord, \DateTimeImmutable $activationDate, \DateTimeImmutable $windowEndsOn): void
    {
        $email = (new Email())
            ->from(self::FROM)
            ->to($paymentRecord->getContactEmail())
            ->subject('Unlimited access subscription activated')
            ->text(sprintf(
                "Your unlimited access subscription is active from %s through %s.",
                $activationDate->format('Y-m-d'),
                $windowEndsOn->format('Y-m-d'),
            ));

        $this->mailer->send($email);
    }
}
