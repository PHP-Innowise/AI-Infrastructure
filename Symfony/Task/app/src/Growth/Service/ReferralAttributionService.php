<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Entity\Referral;
use App\Growth\Entity\ReferralLink;
use App\Growth\Repository\ReferralLinkRepository;
use App\Growth\Repository\ReferralRepository;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\HttpFoundation\Cookie;
use Symfony\Component\HttpFoundation\Request;

/**
 * BR-06-1/2/3: the click -> cookie -> registration-completes-attribution
 * workflow. Two halves, called from two different, unrelated moments:
 *
 * - `buildAttributionCookie()` — the public join controller, on a link
 *   click (AC-06-4/5).
 * - `completeAttribution()` — `ReferralRegistrationSubscriber`, once a new
 *   account has registered (AC-06-5/6/7).
 *
 * No server-side click log exists for referral links (unlike Epic-01's
 * `ShareLinkOpen`) — the cookie itself IS the record of the click, per
 * `specs/database-designer-schema.md` "`referral_link`"'s own design: BR-06-2
 * states attribution is "tracked via a browser cookie," and `referral.
 * clicked_at` is filled in "from the 30-day attribution cookie" only once a
 * registration actually happens. A click that never converts to a
 * registration leaves no trace anywhere, which matches the epic's own
 * silence on tracking abandoned clicks.
 */
final readonly class ReferralAttributionService
{
    public const COOKIE_NAME = 'pp_referral_attribution';
    private const COOKIE_TTL_DAYS = 30;

    public function __construct(
        private EntityManagerInterface $entityManager,
        private ReferralLinkRepository $referralLinks,
        private ReferralRepository $referrals,
        private ReferralRuleService $referralRules,
    ) {
    }

    /**
     * BR-06-2: last-click wins — simply overwriting any earlier cookie with
     * this one IS the "last click" rule; nothing needs to compare against
     * the previous value. Stored for 30 days regardless of the
     * (configurable) attribution window used at read time, matching the
     * epic's own stated cookie lifetime (US-06.02, "referrer information is
     * stored in a browser cookie for 30 days") as a fixed fact distinct
     * from Q-06.12's configurable window.
     */
    public function buildAttributionCookie(ReferralLink $link, Request $request): Cookie
    {
        $payload = json_encode([
            'referralLinkId' => $link->getId(),
            'trainerId' => $link->getTrainer()->getId(),
            'clickedAt' => (new \DateTimeImmutable())->format(\DATE_ATOM),
        ], \JSON_THROW_ON_ERROR);

        return Cookie::create(self::COOKIE_NAME)
            ->withValue($payload)
            ->withExpires(new \DateTimeImmutable(sprintf('+%d days', self::COOKIE_TTL_DAYS)))
            ->withHttpOnly(true)
            ->withSameSite(Cookie::SAMESITE_LAX)
            ->withSecure($request->isSecure())
            ->withPath('/');
    }

    /**
     * AC-06-5/6/7: reads the attribution cookie (if any) and records the
     * referral. Every failure mode is a silent no-op — a missing, expired,
     * malformed, cross-trainer, or self-referral cookie never blocks or
     * errors the registration that is already complete by the time this
     * runs; it simply means no referral credit is given, exactly as
     * US-06.02's own Edge Cases specify.
     */
    public function completeAttribution(Request $request, Trainer $registeredTrainer, PlayerProfile $refereePlayer): ?Referral
    {
        $cookieValue = $request->cookies->get(self::COOKIE_NAME);

        if (!\is_string($cookieValue) || '' === $cookieValue) {
            return null;
        }

        $payload = $this->decodeCookiePayload($cookieValue);

        if (null === $payload) {
            return null;
        }

        // AC-06-7: registered with a different trainer than the one whose
        // link was clicked -> no credit.
        if ((int) $payload['trainerId'] !== $registeredTrainer->getId()) {
            return null;
        }

        $link = $this->referralLinks->find((int) $payload['referralLinkId']);

        if (null === $link || $link->getTrainer()->getId() !== $registeredTrainer->getId()) {
            return null;
        }

        // BR-06-3: a player cannot refer themselves.
        if ($link->getPlayer()->getId() === $refereePlayer->getId()) {
            return null;
        }

        // BR-06-3: a referee is credited to exactly one referral, ever —
        // also the "friend already has an account" edge case: the caller
        // only invokes this for a BRAND NEW registration, so an existing
        // account clicking the link again never reaches this method at all.
        if (null !== $this->referrals->findOneByRefereePlayer($refereePlayer)) {
            return null;
        }

        try {
            $clickedAt = new \DateTimeImmutable((string) $payload['clickedAt']);
        } catch (\Exception) {
            return null;
        }

        $registeredAt = new \DateTimeImmutable();

        // AC-06-6/BR-06-1: the (configurable) attribution window, measured
        // from click to registration.
        $windowDays = $this->referralRules->current()->attributionWindowDays;

        if ($clickedAt->modify(sprintf('+%d days', $windowDays)) < $registeredAt) {
            return null;
        }

        $referral = new Referral($registeredTrainer, $link, $link->getPlayer(), $refereePlayer, $clickedAt, $registeredAt);
        $this->referrals->add($referral);
        $this->entityManager->flush();

        return $referral;
    }

    /**
     * @return array{referralLinkId: int, trainerId: int, clickedAt: string}|null
     */
    private function decodeCookiePayload(string $cookieValue): ?array
    {
        try {
            /** @var mixed $payload */
            $payload = json_decode($cookieValue, true, flags: \JSON_THROW_ON_ERROR);
        } catch (\JsonException) {
            return null;
        }

        if (!\is_array($payload)
            || !isset($payload['referralLinkId'], $payload['trainerId'], $payload['clickedAt'])
            || !is_numeric($payload['referralLinkId'])
            || !is_numeric($payload['trainerId'])
            || !\is_string($payload['clickedAt'])
        ) {
            return null;
        }

        return [
            'referralLinkId' => (int) $payload['referralLinkId'],
            'trainerId' => (int) $payload['trainerId'],
            'clickedAt' => $payload['clickedAt'],
        ];
    }
}
