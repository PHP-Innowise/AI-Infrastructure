<?php

declare(strict_types=1);

namespace App\Billing\Service;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Entity\TokenPackage;
use App\Billing\Repository\PaymentRecordRepository;
use App\Billing\Repository\TokenPackageRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Service\ChildActionAttempt;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\FundingMethod;
use App\Identity\Voter\ChildApprovalVoter;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\SecurityBundle\Security;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * US-05.02: buying tokens — a listed package, or a custom token count
 * (modelled as a one-off, unlisted `TokenPackage` — see that entity's own
 * docblock). Always `card`-funded (BR-05-6: "all USD payments go through
 * Stripe Checkout") — there is no "buy tokens with tokens."
 *
 * AC-05-6: a child-initiated purchase queues for parent approval exactly
 * like `PurchasePlaylistAccessService`'s own shape; the actual Checkout
 * redirect only happens once bypass is granted (an adult acting directly,
 * or the parent's own approval).
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-4..6, BR-05-1
 */
final readonly class TokenPurchaseService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TokenPackageRepository $tokenPackages,
        private PaymentRecordRepository $paymentRecords,
        private TrainerBillingSettingsRepository $billingSettings,
        private StripeGateway $stripeGateway,
        private Security $security,
        private ChildApprovalService $childApprovalService,
        private UrlGeneratorInterface $urlGenerator,
    ) {
    }

    /**
     * Returns the Checkout URL to redirect to, or null if the attempt was
     * queued for parent approval instead.
     */
    public function purchasePackage(Trainer $trainer, PlayerProfile $player, Account $actor, TokenPackage $package): ?string
    {
        $bypassGranted = $this->checkBypass($actor, $player);

        if (!$bypassGranted) {
            $this->childApprovalService->requestApproval(
                $trainer,
                $player,
                ChildApprovalRequest::ACTION_TOKEN_PURCHASE,
                requestedTokenPackageId: (int) $package->getId(),
            );

            return null;
        }

        return $this->startCheckout($trainer, $actor, $package);
    }

    /**
     * AC-05-4: "or a custom amount" — priced at the trainer's own
     * $/token rate, packaged as a one-off `TokenPackage` row (see that
     * entity's own docblock for why, rather than a parallel un-packaged
     * code path).
     */
    public function purchaseCustomAmount(Trainer $trainer, PlayerProfile $player, Account $actor, int $tokenCount): ?string
    {
        if ($tokenCount <= 0) {
            throw new \InvalidArgumentException('A custom token purchase must be for at least one token.');
        }

        $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
        $package = new TokenPackage(
            $trainer,
            sprintf('Custom: %d tokens', $tokenCount),
            $tokenCount,
            $tokenCount * $settings->getTokenPriceMinorUnits(),
            isActive: false,
        );
        $this->tokenPackages->add($package);
        $this->entityManager->flush();

        return $this->purchasePackage($trainer, $player, $actor, $package);
    }

    /**
     * AC-05-6: the parent approved a pending ACTION_TOKEN_PURCHASE request —
     * always proceeds straight to Checkout (the approval itself IS the
     * bypass), so, unlike purchasePackage(), this never returns null.
     */
    public function completeAfterParentApproval(Trainer $trainer, PlayerProfile $player, Account $payer, TokenPackage $package): string
    {
        return $this->startCheckout($trainer, $payer, $package);
    }

    private function checkBypass(Account $actor, PlayerProfile $player): bool
    {
        // BR-05-1/AC-05-4: token purchases are always card-funded — never
        // "token" (there is no such thing as buying tokens with tokens).
        return $this->security->isGranted(
            ChildApprovalVoter::CHILD_APPROVAL_BYPASS,
            new ChildActionAttempt($actor, $player, FundingMethod::Usd),
        );
    }

    private function startCheckout(Trainer $trainer, Account $payer, TokenPackage $package): string
    {
        $paymentRecord = new PaymentRecord(
            $trainer,
            PaymentRecord::TYPE_TOKEN_PURCHASE,
            PaymentRecord::METHOD_CARD,
            $package->getPriceMinorUnits(),
            $this->contactNameFor($payer),
            $payer->getEmail(),
            $payer,
        );
        $paymentRecord->attachRelatedTokenPackage($package);
        $this->stripeGateway->applyCurrentFee($paymentRecord);
        $this->paymentRecords->add($paymentRecord);
        $this->entityManager->flush();

        $session = $this->stripeGateway->createCheckoutSession(
            $paymentRecord,
            $this->urlGenerator->generate('billing_portal_checkout_success', ['context' => 'tokens'], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_portal_checkout_cancel', ['context' => 'tokens'], UrlGeneratorInterface::ABSOLUTE_URL),
        );

        return $session->checkoutUrl;
    }

    private function contactNameFor(Account $account): string
    {
        $profile = $account->getProfile();

        return null !== $profile ? trim($profile->getFirstName().' '.$profile->getLastName()) : $account->getEmail();
    }
}
