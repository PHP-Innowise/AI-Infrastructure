<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Form\ReferralRuleType;
use App\Growth\Service\ReferralRuleService;
use App\Identity\Entity\Account;
use App\Platform\Voter\PlatformConfigurationVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-06.08: Super Admin configures the platform-wide referral reward rule —
 * "System Settings" -> "Referral Program".
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-29..31
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class SuperAdminReferralRuleController extends AbstractController
{
    public function __construct(
        private readonly ReferralRuleService $referralRules,
    ) {
    }

    #[Route('/super-admin/growth/referral-rules', name: 'growth_super_admin_referral_rules', methods: ['GET', 'POST'])]
    public function edit(Request $request): Response
    {
        $this->denyAccessUnlessGranted(PlatformConfigurationVoter::PLATFORM_CONFIG_EDIT, null);

        $current = $this->referralRules->current();
        $form = $this->createForm(ReferralRuleType::class, [
            'referralsRequired' => $current->referralsRequired,
            'tokensAwarded' => $current->tokensAwarded,
            'refereeWelcomeBonus' => $current->refereeWelcomeBonus,
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{referralsRequired: int, tokensAwarded: int, refereeWelcomeBonus: bool} $data */
            $data = $form->getData();

            try {
                // AC-06-30: applies to every trainer immediately, existing
                // assist counts preserved. AC-06-31: audit-logged with old
                // and new values — both handled inside the service.
                $this->referralRules->update($data['referralsRequired'], $data['tokensAwarded'], $data['refereeWelcomeBonus'], $this->actor());
                $this->addFlash('success', 'Referral rule updated. The new rule applies to every trainer immediately.');

                return $this->redirectToRoute('growth_super_admin_referral_rules');
            } catch (\InvalidArgumentException $e) {
                $form->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('growth/super_admin_referral_rules.html.twig', [
            'form' => $form,
            'current' => $current,
        ]);
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
