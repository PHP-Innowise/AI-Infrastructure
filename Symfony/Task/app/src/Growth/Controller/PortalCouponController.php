<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Content\Repository\PlaylistRepository;
use App\Growth\Entity\Coupon;
use App\Growth\Service\CouponPricingService;
use App\Growth\Voter\CouponVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\EventRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\JsonResponse;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-06.06: the "Have a coupon code?" preview call — a UX convenience only,
 * never the authoritative check (`specs/api-designer-spec.md`: "re-
 * validated again at final checkout submission inside the owning route").
 * The owning routes (`scheduling_portal_event_rsvp`,
 * `content_portal_playlist_checkout`) call `CouponPricingService::quote()`
 * themselves before ever building a payment request — this controller calls
 * the exact same service method, just earlier and non-bindingly.
 *
 * Reads `EventRepository`/`PlaylistRepository` directly to resolve the
 * purchase's original price — a READ through the owning module's own
 * repository, which `architect-architecture.md`'s "Boundary rule"
 * explicitly allows ("a module may read another module's entities through
 * that module's repository or service"); only a WRITE is restricted to the
 * owning module's service.
 *
 * @see specs/api-designer-spec.md "Growth module" (`growth_portal_coupon_validate`)
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalCouponController extends AbstractController
{
    private const PURCHASE_TYPE_RSVP = 'rsvp';
    private const PURCHASE_TYPE_CONTENT = 'content';

    public function __construct(
        private readonly CouponPricingService $couponPricing,
        private readonly EventRepository $events,
        private readonly PlaylistRepository $playlists,
        private readonly PlayerContextResolver $playerContext,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    #[Route('/portal/checkout/coupon/validate', name: 'growth_portal_coupon_validate', methods: ['POST'])]
    public function validate(Request $request): JsonResponse
    {
        /** @var array{code?: mixed, purchaseType?: mixed, purchaseId?: mixed}|null $payload */
        $payload = json_decode($request->getContent(), true);
        $payload ??= [];

        $code = \is_string($payload['code'] ?? null) ? $payload['code'] : '';
        $purchaseType = \is_string($payload['purchaseType'] ?? null) ? $payload['purchaseType'] : '';
        $purchaseId = is_numeric($payload['purchaseId'] ?? null) ? (int) $payload['purchaseId'] : null;

        $trainer = $this->currentTrainer();
        $player = $this->currentPlayer($request);

        $resolved = $this->resolveOriginalAmountAndCouponScope($trainer, $purchaseType, $purchaseId);

        if (null === $resolved) {
            return $this->json(['valid' => false, 'message' => 'Invalid or expired code']);
        }

        [$originalAmountMinorUnits, $appliesToScope] = $resolved;

        $quote = $this->couponPricing->quote($trainer, $code, $player, $appliesToScope, $originalAmountMinorUnits);

        if (!$quote->valid) {
            return $this->json(['valid' => false, 'message' => $quote->message]);
        }

        \assert($quote->coupon instanceof Coupon);

        if (!$this->isGranted(CouponVoter::COUPON_APPLY, $quote->coupon)) {
            return $this->json(['valid' => false, 'message' => 'Invalid or expired code']);
        }

        return $this->json([
            'valid' => true,
            'discountedAmountCents' => $quote->finalAmountMinorUnits,
            'message' => $quote->message,
        ]);
    }

    /**
     * @return array{0: int, 1: string}|null
     */
    private function resolveOriginalAmountAndCouponScope(Trainer $trainer, string $purchaseType, ?int $purchaseId): ?array
    {
        if (null === $purchaseId) {
            return null;
        }

        if (self::PURCHASE_TYPE_RSVP === $purchaseType) {
            $event = $this->events->find($purchaseId);

            if (null === $event || $event->getTrainer()->getId() !== $trainer->getId()) {
                return null;
            }

            return [$event->priceForMethod(Event::PAYMENT_USD), Coupon::APPLIES_TO_EVENTS];
        }

        if (self::PURCHASE_TYPE_CONTENT === $purchaseType) {
            $playlist = $this->playlists->find($purchaseId);

            if (null === $playlist || $playlist->getTrainer()->getId() !== $trainer->getId()) {
                return null;
            }

            return [$playlist->priceForMethod('usd'), Coupon::APPLIES_TO_CONTENT];
        }

        return null;
    }

    private function currentPlayer(Request $request): PlayerProfile
    {
        return $this->playerContext->resolve($request, $this->actor());
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
