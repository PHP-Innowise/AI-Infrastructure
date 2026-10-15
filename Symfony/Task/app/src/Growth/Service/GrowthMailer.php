<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;

/**
 * Every Epic-06 transactional email, matching
 * `App\Billing\Service\BillingMailer`'s own precedent — plain-text `Email`,
 * not `TemplatedEmail` + Twig.
 */
final readonly class GrowthMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
    ) {
    }

    /**
     * AC-06-10: "You earned 1 token! Your friend [Name] just joined."
     */
    public function sendReferralRewardEarned(Account $referrerAccount, PlayerProfile $refereePlayer, Trainer $trainer, int $tokensAwarded): void
    {
        $email = (new Email())
            ->from(self::FROM)
            ->to($referrerAccount->getEmail())
            ->subject(sprintf('You earned %d token%s!', $tokensAwarded, 1 === $tokensAwarded ? '' : 's'))
            ->text(sprintf(
                "You earned %d token%s! Your friend %s just joined %s.",
                $tokensAwarded,
                1 === $tokensAwarded ? '' : 's',
                $refereePlayer->getFirstName(),
                $trainer->getBusinessName(),
            ));

        $this->mailer->send($email);
    }
}
