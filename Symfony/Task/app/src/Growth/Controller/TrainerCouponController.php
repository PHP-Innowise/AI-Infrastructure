<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Entity\Coupon;
use App\Growth\Exception\CouponInUseException;
use App\Growth\Exception\DuplicateCouponCodeException;
use App\Growth\Form\CouponType;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Growth\Service\CouponAnalyticsService;
use App\Growth\Service\CouponService;
use App\Growth\Voter\CouponVoter;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\Form\FormError;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-06.05/06.07: coupon creation, edit, deactivation, deletion, and
 * analytics — "Marketing" -> "Coupons".
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md AC-06-17..19, AC-06-26..28
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerCouponController extends AbstractController
{
    public function __construct(
        private readonly CouponService $couponService,
        private readonly CouponAnalyticsService $analytics,
        private readonly CouponRedemptionRepository $redemptions,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-06-26/28: the coupon list plus the all-coupons analytics summary,
     * one screen (matching `growth_trainer_coupons_index`'s combined
     * response in `specs/api-designer-spec.md`).
     */
    #[Route('/trainer/marketing/coupons', name: 'growth_trainer_coupons_index', methods: ['GET'])]
    public function index(): Response
    {
        $trainer = $this->currentTrainer();
        $now = new \DateTimeImmutable();
        $monthStart = $now->modify('first day of this month')->setTime(0, 0);
        $monthEnd = $monthStart->modify('+1 month');

        return $this->render('growth/trainer_coupons_index.html.twig', [
            'rows' => $this->analytics->allRows(),
            'summary' => $this->analytics->summary($trainer, $monthStart, $monthEnd),
        ]);
    }

    #[Route('/trainer/marketing/coupons/new', name: 'growth_trainer_coupon_create', methods: ['GET', 'POST'])]
    public function create(Request $request): Response
    {
        $this->denyAccessUnlessGranted(CouponVoter::COUPON_CREATE, null);

        $form = $this->createForm(CouponType::class, ['isActive' => true]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{code: string, discountType: string, discountValue: int, appliesTo: string, usageLimit: ?int, eligibility: string, expiresAt: ?\DateTimeImmutable, isActive: bool} $data */
            $data = $form->getData();

            try {
                $this->couponService->create(
                    $this->currentTrainer(),
                    $data['code'],
                    $data['discountType'],
                    $data['discountValue'],
                    $data['appliesTo'],
                    $data['usageLimit'],
                    $data['eligibility'],
                    $data['expiresAt'],
                    $data['isActive'],
                    $this->actor(),
                );
                $this->addFlash('success', 'Coupon created.');

                return $this->redirectToRoute('growth_trainer_coupons_index');
            } catch (DuplicateCouponCodeException $e) {
                $form->get('code')->addError(new FormError($e->getMessage()));
            } catch (\InvalidArgumentException $e) {
                $form->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('growth/trainer_coupon_form.html.twig', ['form' => $form, 'mode' => 'create']);
    }

    #[Route('/trainer/marketing/coupons/{coupon}/edit', name: 'growth_trainer_coupon_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Coupon $coupon): Response
    {
        $this->denyAccessUnlessGranted(CouponVoter::COUPON_EDIT, $coupon);

        $form = $this->createForm(CouponType::class, [
            'code' => $coupon->getCode(),
            'discountType' => $coupon->getDiscountType(),
            'discountValue' => $coupon->getDiscountValue(),
            'appliesTo' => $coupon->getAppliesTo(),
            'usageLimit' => $coupon->getUsageLimit(),
            'eligibility' => $coupon->getEligibility(),
            'expiresAt' => $coupon->getExpiresAt(),
            'isActive' => $coupon->isActive(),
        ], ['isEdit' => true]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{usageLimit: ?int, expiresAt: ?\DateTimeImmutable, isActive: bool} $data */
            $data = $form->getData();

            try {
                // AC-06-27: only expiration, usage limit, and status are
                // ever actually applied — see CouponType's own docblock.
                $this->couponService->update($coupon, $data['expiresAt'], $data['usageLimit'], $data['isActive']);
                $this->addFlash('success', 'Coupon updated.');

                return $this->redirectToRoute('growth_trainer_coupons_index');
            } catch (\InvalidArgumentException $e) {
                $form->addError(new FormError($e->getMessage()));
            }
        }

        return $this->render('growth/trainer_coupon_form.html.twig', ['form' => $form, 'mode' => 'edit', 'coupon' => $coupon]);
    }

    #[Route('/trainer/marketing/coupons/{coupon}/deactivate', name: 'growth_trainer_coupon_deactivate', methods: ['POST'])]
    public function deactivate(Request $request, Coupon $coupon): Response
    {
        $this->denyAccessUnlessGranted(CouponVoter::COUPON_DEACTIVATE, $coupon);

        if (!$this->isCsrfTokenValid('coupon-deactivate'.$coupon->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->couponService->deactivate($coupon);
        $this->addFlash('success', sprintf('Coupon "%s" deactivated.', $coupon->getCode()));

        return $this->redirectToRoute('growth_trainer_coupons_index');
    }

    #[Route('/trainer/marketing/coupons/{coupon}/delete', name: 'growth_trainer_coupon_delete', methods: ['POST'])]
    public function delete(Request $request, Coupon $coupon): Response
    {
        $this->denyAccessUnlessGranted(CouponVoter::COUPON_DELETE, $coupon);

        if (!$this->isCsrfTokenValid('coupon-delete'.$coupon->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        try {
            $code = $coupon->getCode();
            $this->couponService->delete($coupon);
            $this->addFlash('success', sprintf('Coupon "%s" deleted.', $code));
        } catch (CouponInUseException $e) {
            $this->addFlash('error', $e->getMessage());
        }

        return $this->redirectToRoute('growth_trainer_coupons_index');
    }

    /**
     * AC-06-27: "view usage details (the list of players who used it)."
     */
    #[Route('/trainer/marketing/coupons/{coupon}/usage', name: 'growth_trainer_coupon_usage', methods: ['GET'])]
    public function usage(Coupon $coupon): Response
    {
        $this->denyAccessUnlessGranted(CouponVoter::COUPON_VIEW_ANALYTICS, $coupon);

        return $this->render('growth/trainer_coupon_usage.html.twig', [
            'coupon' => $coupon,
            'redemptions' => $this->redemptions->findAllForCoupon($coupon),
        ]);
    }

    private function currentTrainer(): Trainer
    {
        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $this->tenantContext->requireTrainerId());

        return $trainer;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
