<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\AdministrativeScope;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\RedirectResponse;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-07.09: Super Admin links out to Stripe rather than the platform
 * duplicating any financial reporting — AC-07-6, AC-07-35..37. The earnings
 * boundary (architect-architecture.md "The earnings boundary"; AC-07-7)
 * means neither route here ever reads or displays a money figure — both
 * are pure redirects.
 *
 * **No real Stripe API call for either link** — matches
 * `TrainerBillingController`'s own established precedent for "Stripe
 * Express Dashboard" links (`templates/billing/trainer_earnings.html.twig`:
 * a plain `https://dashboard.stripe.com` href, no `login_link` API call),
 * itself forced by `STRIPE_SECRET_KEY` being deliberately blank in every
 * environment this codebase runs in (`Billing/Stripe/StripeClient.php`'s
 * own docblock). AC-07-35's "auto-logging in via SSO if the session is
 * active" describes Stripe's own browser session, not something this
 * platform can drive without a real key — there is nothing to fake here
 * without inventing a `StripeClient::createLoginLink()` method with no
 * other caller in this codebase.
 *
 * @see specs/api-designer-spec.md "Administration module" — administration_stripe_dashboard_link, administration_trainer_stripe_dashboard_link
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-6, AC-07-35..37
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class StripeDashboardLinkController extends AbstractController
{
    /**
     * The platform owner's own Stripe account — not a Connect/Express
     * relationship at all (there is no "connected account id" for the
     * platform's own account anywhere in this schema), so this is a plain
     * link to Stripe's own dashboard home, exactly like
     * `trainer_earnings.html.twig`'s existing link.
     */
    private const PLATFORM_DASHBOARD_URL = 'https://dashboard.stripe.com';

    public function __construct(
        private readonly TrainerBillingSettingsRepository $billingSettings,
        private readonly AdministrativeScope $administrativeScope,
    ) {
    }

    /**
     * AC-07-6, AC-07-35, AC-07-36: "View Financial Reports in Stripe."
     */
    #[Route('/super-admin/stripe', name: 'administration_stripe_dashboard_link', methods: ['GET'])]
    public function platform(): RedirectResponse
    {
        return new RedirectResponse(self::PLATFORM_DASHBOARD_URL);
    }

    /**
     * AC-07-37: "View [Trainer]'s Stripe Account" — that trainer's own
     * Connect account, addressed directly by id
     * (`https://dashboard.stripe.com/{connected_account_id}` is Stripe's
     * own real URL shape for viewing a connected account from the platform
     * side — no API call needed to construct it). `TrainerBillingSettings`
     * is trainer-scoped (RLS), so reading it needs a genuine tenant context
     * first — the same `AdministrativeScope` mechanism
     * `TrainerFeeController` already opens for this exact reason.
     */
    #[Route('/super-admin/trainers/{trainer}/stripe', name: 'administration_trainer_stripe_dashboard_link', methods: ['GET'])]
    public function trainer(Trainer $trainer): RedirectResponse
    {
        $this->administrativeScope->openFor($trainer, $this->actor());

        try {
            $settings = $this->billingSettings->getOrCreateForTrainer($trainer);
            $accountId = $settings->getStripeConnectAccountId();
        } finally {
            $this->administrativeScope->close();
        }

        if (null === $accountId) {
            $this->addFlash('error', sprintf('%s has not connected Stripe yet.', $trainer->getBusinessName()));

            return $this->redirectToRoute('administration_trainer_show', ['trainer' => $trainer->getId()]);
        }

        return new RedirectResponse(sprintf('%s/%s', self::PLATFORM_DASHBOARD_URL, $accountId));
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
