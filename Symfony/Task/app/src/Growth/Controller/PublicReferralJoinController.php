<?php

declare(strict_types=1);

namespace App\Growth\Controller;

use App\Growth\Service\ReferralAttributionService;
use App\Growth\Service\ReferralLinkService;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;

/**
 * AC-06-1/4/5: the public referral-link landing — `platform.com/join/
 * {trainer-slug}/{player-id}`. A friend clicking a shared referral link
 * lands here, gets the 30-day attribution cookie (BR-06-1/2), and is taken
 * straight to that trainer's own registration page (AC-06-4) — reusing
 * Identity's already-built static ShareLink registration flow rather than
 * duplicating it (Growth "may call Identity" for reads; it must never
 * create a membership or account itself — that stays `Identity`'s
 * exclusive job via `MembershipService`/`PlayerRegistrationService`).
 *
 * **Tenant activation happens explicitly here**, not via
 * `TenantFromPublicCode`'s allow-list: that mechanism is keyed to an opaque
 * `PublicTenantCode` looked up by a `{code}` route parameter
 * (`specs/architect-architecture.md` "Tenancy enforcement," resolution
 * source 5); a trainer slug is neither opaque nor registered there — Growth
 * introduces no new `PublicTenantCode` rows, since nothing about this
 * route's parameters is secret or needs revocation. `TenantContext::
 * activateFor()` is a plain public method exactly as available to this
 * controller as it is to the resolver itself; calling it directly, after
 * independently verifying the slug resolves to a real trainer, keeps
 * Platform's core resolver untouched rather than widening its allow-list
 * mechanism for a shape it was not designed to carry.
 *
 * This is the analyst-flagged gap between Epic-06's stated dependency on
 * Epic-01's ShareLink system and its own, differently-shaped referral link
 * — `specs/requirements-analyst-epic-06-marketing-growth-spec.md` "Open
 * questions" leaves resolving it to the architect stage; this codebase's
 * resolution is a second, Growth-owned `ReferralLink`/`Referral` mechanism
 * (matching `specs/database-designer-schema.md`'s own settled table design)
 * that hands off to ShareLink only for the registration FORM itself.
 */
final class PublicReferralJoinController extends AbstractController
{
    public function __construct(
        private readonly TrainerRepository $trainers,
        private readonly PlayerProfileRepository $playerProfiles,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly ShareLinkRepository $shareLinks,
        private readonly ReferralLinkService $referralLinkService,
        private readonly ReferralAttributionService $attribution,
        private readonly TenantContext $tenantContext,
    ) {
    }

    #[Route('/join/{trainerSlug}/{playerId<\d+>}', name: 'growth_public_referral_join', methods: ['GET'])]
    public function __invoke(string $trainerSlug, int $playerId, Request $request): Response
    {
        $trainer = $this->trainers->findOneBySlug($trainerSlug);

        if (null === $trainer) {
            throw $this->createNotFoundException();
        }

        $this->tenantContext->activateFor($trainer);

        $referrer = $this->playerProfiles->find($playerId);

        if (null === $referrer) {
            throw $this->createNotFoundException();
        }

        // The referral link only means something for a player genuinely,
        // actively associated with this trainer — an id that resolves to
        // SOME player but never joined (or has since left) this trainer
        // 404s exactly like an unknown player would.
        $membership = $this->memberships->findOneByTrainerAndPlayer($trainer, $referrer);

        if (null === $membership || !$membership->isActive()) {
            throw $this->createNotFoundException();
        }

        // AC-06-1: lazily provisioned if this is the very first time
        // anyone (including the referrer themselves, from their own
        // dashboard) has ever resolved this player's link.
        $link = $this->referralLinkService->linkFor($trainer, $referrer);
        $cookie = $this->attribution->buildAttributionCookie($link, $request);

        $staticLink = $this->shareLinks->findStaticPlayerLink((int) $trainer->getId());
        $redirectUrl = null !== $staticLink
            ? $this->generateUrl('identity_sharelink_show', ['code' => $staticLink->getCode()])
            : $this->generateUrl('identity_auth_login');

        $response = $this->redirect($redirectUrl);
        $response->headers->setCookie($cookie);

        return $response;
    }
}
