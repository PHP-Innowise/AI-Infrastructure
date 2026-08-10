<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\ShareLink;
use App\Identity\Entity\ShareLinkOpen;
use App\Identity\Repository\ShareLinkOpenRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Entity\PublicTenantCode;
use App\Platform\Entity\Trainer;
use App\Platform\Service\PublicTenantCodeRegistry;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Issue, resolve-support, and revoke ShareLinks (AC-01-9, AC-01-73, BR-01-14,
 * BR-01-15, BR-01-27). Owns the one invariant that must never drift: every
 * ShareLink this issues gets a matching `PublicTenantCode` row in the same
 * transaction, or `/join/{code}` and `/invite/{code}` 404 for a code that
 * genuinely exists.
 *
 * @see specs/database-designer-schema.md "`share_link`"
 */
final readonly class ShareLinkService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ShareLinkRepository $shareLinks,
        private ShareLinkOpenRepository $shareLinkOpens,
        private PublicTenantCodeRegistry $publicTenantCodes,
    ) {
    }

    /**
     * AC-01-73: static, unlimited-use, no-expiry — every trainer gets exactly
     * one, provisioned alongside the trainer itself.
     */
    public function issueStaticPlayerLink(Trainer $trainer, Account $createdBy): ShareLink
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $createdBy): ShareLink {
            $code = $this->generateCode();
            $link = new ShareLink($trainer, $code, ShareLink::TYPE_STATIC_PLAYER, $createdBy);
            $this->shareLinks->add($link);
            $this->entityManager->flush();

            $this->publicTenantCodes->issue($code, $trainer, PublicTenantCode::KIND_SHARELINK, (int) $link->getId());

            return $link;
        });
    }

    /**
     * AC-01-39, BR-01-15: unique, one-time-use, 7-day expiry, addressed to a
     * specific email.
     */
    public function issueCoachInvite(Trainer $trainer, Account $createdBy, string $targetEmail): ShareLink
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $createdBy, $targetEmail): ShareLink {
            $code = $this->generateCode();
            $link = new ShareLink($trainer, $code, ShareLink::TYPE_UNIQUE_COACH, $createdBy, $targetEmail);
            $this->shareLinks->add($link);
            $this->entityManager->flush();

            $this->publicTenantCodes->issue($code, $trainer, PublicTenantCode::KIND_SHARELINK, (int) $link->getId());

            return $link;
        });
    }

    /**
     * AC-01-42: reissues a fresh 7-day window under the SAME code, so the
     * PublicTenantCode mapping (and any link already shared) keeps working.
     */
    public function resend(ShareLink $link): ShareLink
    {
        $link->renew(new \DateTimeImmutable());

        return $link;
    }

    public function revoke(ShareLink $link): void
    {
        $link->revoke();
        $this->publicTenantCodes->revoke(PublicTenantCode::KIND_SHARELINK, (int) $link->getId());
    }

    /**
     * BR-03-22: the click log. Runs under whatever tenant is already active
     * for the code (the ambient resolver, on an allow-listed route). Flushes
     * itself: on a plain GET show-page hit, this is frequently the only
     * write in the whole request, so nothing else would ever commit it.
     */
    public function recordOpen(ShareLink $link, ?string $ipAddress = null): void
    {
        $this->shareLinkOpens->add(new ShareLinkOpen($link->getTrainer(), $link, $ipAddress));
        $this->entityManager->flush();
    }

    /**
     * BR-01-27: usage count per link. Flushes itself: callers frequently call
     * this immediately after a membership-creation call that has already
     * flushed and committed its own transaction (`MembershipService`'s
     * methods each own their own transaction), so this mutation cannot rely
     * on a later flush elsewhere to persist it.
     */
    public function recordUse(ShareLink $link): void
    {
        $link->recordUse();
        $this->entityManager->flush();
    }

    /**
     * High-entropy, URL-safe, 12 characters (72 bits of randomness).
     */
    public function generateCode(): string
    {
        return rtrim(strtr(base64_encode(random_bytes(9)), '+/', '-_'), '=');
    }
}
