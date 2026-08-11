<?php

declare(strict_types=1);

namespace App\Billing\Controller;

use App\Billing\Entity\TrainerBillingSettings;
use App\Billing\Form\GiftTokensType;
use App\Billing\Form\TokenPricingType;
use App\Billing\Repository\TokenPackageRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Service\PlayerAccountResolver;
use App\Billing\Service\StripeGateway;
use App\Billing\Service\StripeReportingReader;
use App\Billing\Service\TokenLedgerService;
use App\Billing\Voter\TokenVoter;
use App\Billing\Voter\TrainerSettingsVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-05.01/09/10, AC-05-1..3/25..28/33: the trainer's own Stripe Connect
 * status, pricing, gifting, and simplified earnings summary.
 *
 * @see specs/api-designer-spec.md "Billing module" — trainer console table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerBillingController extends AbstractController
{
    public function __construct(
        private readonly TrainerBillingSettingsRepository $billingSettings,
        private readonly TokenPackageRepository $tokenPackages,
        private readonly StripeGateway $stripeGateway,
        private readonly StripeReportingReader $earningsReader,
        private readonly TokenLedgerService $tokenLedger,
        private readonly PlayerAccountResolver $playerAccounts,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly UrlGeneratorInterface $urlGenerator,
    ) {
    }

    #[Route('/trainer/billing', name: 'billing_trainer_settings', methods: ['GET'])]
    public function settings(): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_VIEW, $settings);

        return $this->render('billing/trainer_settings.html.twig', [
            'settings' => $settings,
            'packages' => $this->tokenPackages->findAllForTrainer($this->activeTrainer()),
        ]);
    }

    /**
     * AC-05-1: redirected to Stripe Connect Express onboarding.
     */
    #[Route('/trainer/billing/stripe/connect', name: 'billing_trainer_stripe_connect', methods: ['POST'])]
    public function connect(Request $request): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_EDIT, $settings);

        if (!$this->isCsrfTokenValid('billing-stripe-connect', $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $url = $this->stripeGateway->startConnectOnboarding(
            $this->activeTrainer(),
            $this->urlGenerator->generate('billing_trainer_stripe_return', [], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_trainer_stripe_refresh', [], UrlGeneratorInterface::ABSOLUTE_URL),
        );

        return $this->redirect($url);
    }

    /**
     * AC-05-1: "is redirected back... sees status 'Stripe Connected ✓'" —
     * verified against Stripe's own Account object.
     */
    #[Route('/trainer/billing/stripe/return', name: 'billing_trainer_stripe_return', methods: ['GET'])]
    public function returnFromStripe(): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_EDIT, $settings);

        $refreshed = $this->stripeGateway->refreshConnectStatus($this->activeTrainer());

        $this->addFlash(
            $refreshed->isStripeConnected() ? 'success' : 'info',
            $refreshed->isStripeConnected() ? 'Stripe Connected ✓' : 'Onboarding is not yet complete.',
        );

        return $this->redirectToRoute('billing_trainer_settings');
    }

    /**
     * An expired Account Link is regenerated rather than failing outright.
     */
    #[Route('/trainer/billing/stripe/refresh', name: 'billing_trainer_stripe_refresh', methods: ['GET'])]
    public function refresh(): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_EDIT, $settings);

        $url = $this->stripeGateway->startConnectOnboarding(
            $this->activeTrainer(),
            $this->urlGenerator->generate('billing_trainer_stripe_return', [], UrlGeneratorInterface::ABSOLUTE_URL),
            $this->urlGenerator->generate('billing_trainer_stripe_refresh', [], UrlGeneratorInterface::ABSOLUTE_URL),
        );

        return $this->redirect($url);
    }

    /**
     * BR-05-1, AC-05-1/2/32: the trainer's own $/token price. Blocked
     * ("Connect Stripe first") until Connect completes (AC-05-3).
     */
    #[Route('/trainer/billing/pricing', name: 'billing_trainer_pricing_edit', methods: ['GET', 'POST'])]
    public function pricing(Request $request): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_EDIT, $settings);

        if (!$settings->isStripeConnected()) {
            $this->addFlash('error', 'Connect Stripe first.');

            return $this->redirectToRoute('billing_trainer_settings');
        }

        $form = $this->createForm(TokenPricingType::class, [
            'tokenPriceMinorUnits' => $settings->getTokenPriceMinorUnits(),
            'playerSubscriptionPriceMinorUnits' => $settings->getPlayerSubscriptionPriceMinorUnits(),
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
    /** @var array{tokenPriceMinorUnits: int, playerSubscriptionPriceMinorUnits: ?int} $data */
            $data = $form->getData();
            $settings->updateTokenPrice($data['tokenPriceMinorUnits']);
            $settings->updatePlayerSubscriptionPrice($data['playerSubscriptionPriceMinorUnits']);
            $this->entityManager->flush();

            $this->addFlash('success', 'Pricing updated.');

            return $this->redirectToRoute('billing_trainer_settings');
        }

        return $this->render('billing/trainer_pricing_edit.html.twig', ['form' => $form, 'settings' => $settings]);
    }

    /**
     * AC-05-33: "Trainer Can Gift Tokens" — audit-logged by
     * TokenLedgerService::gift() itself.
     */
    #[Route('/trainer/players/{membership}/gift-tokens', name: 'billing_trainer_gift_tokens', methods: ['POST'])]
    public function giftTokens(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(TokenVoter::TOKEN_GIFT, $membership);

        $form = $this->createForm(GiftTokensType::class);
        $form->handleRequest($request);

        if (!$form->isSubmitted() || !$form->isValid()) {
            $this->addFlash('error', 'Could not process the gift — check the amount and note.');

            return $this->redirectToRoute('billing_trainer_settings');
        }

        /** @var array{amount: int, note: string} $data */
        $data = $form->getData();

        $parentAccount = $this->parentAccountFor($membership);

        if (null === $parentAccount) {
            $this->addFlash('error', 'This player has no parent/self account to credit.');

            return $this->redirectToRoute('billing_trainer_settings');
        }

        $this->tokenLedger->gift($membership->getTrainer(), $parentAccount, $data['amount'], $this->actor(), $data['note']);
        $this->addFlash('success', sprintf('%d tokens gifted.', $data['amount']));

        return $this->redirectToRoute('billing_trainer_settings');
    }

    /**
     * AC-05-25/26: StripeReportingReader-sourced figures only.
     */
    #[Route('/trainer/billing/earnings', name: 'billing_trainer_earnings', methods: ['GET'])]
    public function earnings(): Response
    {
        $settings = $this->settingsForActiveTenant();
        $this->denyAccessUnlessGranted(TrainerSettingsVoter::TRAINER_SETTINGS_VIEW, $settings);

        try {
            $summary = $settings->isStripeConnected() ? $this->earningsReader->earningsSummaryFor($this->activeTrainer()) : null;
            $error = null;
        } catch (\RuntimeException $exception) {
            $summary = null;
            $error = $exception->getMessage();
        }

        return $this->render('billing/trainer_earnings.html.twig', ['summary' => $summary, 'error' => $error, 'settings' => $settings]);
    }

    private function settingsForActiveTenant(): TrainerBillingSettings
    {
        return $this->billingSettings->getOrCreateForTrainer($this->activeTrainer());
    }

    /**
     * The trainer's own console always runs inside its own tenant context
     * (the trainer IS the tenant) — matching
     * `TrainerPlaylistController`/`TrainerEventController`'s own precedent
     * for resolving "the current trainer" via a Doctrine reference, never a
     * query.
     */
    private function activeTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }

    private function parentAccountFor(PlayerTrainerMembership $membership): ?Account
    {
        return $this->playerAccounts->resolve($membership->getPlayer());
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
