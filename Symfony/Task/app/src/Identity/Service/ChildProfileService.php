<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Entity\ShareLink;
use App\Identity\Exception\ShareLinkNotUsableException;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Repository\ShareLinkRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Repository\PublicTenantCodeRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-01.03/US-01.04: child profile creation and the parent's ongoing
 * management of which trainers each child trains with.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.03, US-01.04, AC-01-16..24, AC-01-27/28
 */
final readonly class ChildProfileService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlayerProfileRepository $playerProfiles,
        private ParentChildLinkRepository $parentChildLinks,
        private ShareLinkRepository $shareLinks,
        private PublicTenantCodeRepository $publicTenantCodes,
        private TrainerRepository $trainers,
        private AccountTrainerLinkRepository $accountTrainerLinks,
        private TenantContext $tenantContext,
        private MembershipService $membershipService,
        private ShareLinkService $shareLinkService,
    ) {
    }

    /**
     * AC-01-16/21: creates the child profile. Trainer association (AC-01-17)
     * is a deliberately separate step — the caller passes the trainers to
     * join immediately (possibly none), matching the "otherwise the child
     * profile is created without a trainer association" branch.
     *
     * @param list<Trainer> $trainersToJoin
     */
    public function createChild(
        Account $parent,
        string $firstName,
        \DateTimeImmutable $dateOfBirth,
        ?string $gender,
        ?string $schoolOrTeam,
        array $trainersToJoin = [],
    ): PlayerProfile {
        $age = $dateOfBirth->diff(new \DateTimeImmutable())->y;

        if ($age < 1 || $age > 18) {
            throw new \InvalidArgumentException('A child profile requires an age between 1 and 18.');
        }

        return $this->entityManager->wrapInTransaction(function () use ($parent, $firstName, $dateOfBirth, $gender, $schoolOrTeam, $trainersToJoin): PlayerProfile {
            $child = new PlayerProfile($firstName, $dateOfBirth, null, $gender);

            if (null !== $schoolOrTeam) {
                $child->updateProfile($firstName, $gender, $schoolOrTeam, null, null, null);
            }

            $this->playerProfiles->add($child);
            $this->parentChildLinks->add(new ParentChildLink($parent, $child));
            $this->entityManager->flush();

            foreach ($trainersToJoin as $trainer) {
                $this->addChildToExistingTrainer($parent, $child, $trainer);
            }

            return $child;
        });
    }

    /**
     * AC-01-21: a non-blocking warning only — never prevents creation.
     *
     * @return list<ParentChildLink>
     */
    public function findPossibleDuplicates(Account $parent, string $firstName): array
    {
        return $this->parentChildLinks->findSimilarForParent($parent, $firstName);
    }

    /**
     * AC-01-23 ("select from My Trainers"): the parent already holds an
     * active AccountTrainerLink to this trainer, so no fresh code is needed —
     * the child joins through the trainer's own static player ShareLink,
     * exactly as if they had clicked it (BR-01-27 stays accurate: a real
     * ShareLink is still the recorded source).
     */
    public function addChildToExistingTrainer(Account $parent, PlayerProfile $child, Trainer $trainer): PlayerTrainerMembership
    {
        if (!$this->accountTrainerLinks->isActiveLink($parent, (int) $trainer->getId())) {
            throw new \InvalidArgumentException('This account has no active relationship with that trainer.');
        }

        $this->tenantContext->activateFor($trainer);
        $staticLink = $this->shareLinks->findStaticPlayerLink((int) $trainer->getId());

        $membership = $this->membershipService->associatePlayer($trainer, $child, PlayerTrainerMembership::SOURCE_SHARELINK, $staticLink);

        if (null !== $staticLink) {
            $this->shareLinkService->recordUse($staticLink);
        }

        return $membership;
    }

    /**
     * AC-01-23 ("enter a ShareLink manually"). The code arrives in a POST
     * body, not a route path, so the ambient per-request resolver (which only
     * reads `$request->attributes->get('code')`) never sees it — this method
     * performs the same pre-tenant lookup + activation the ambient resolver
     * would have done for a code-in-path route. See MembershipService's own
     * docblock for why this pattern is safe.
     *
     * @throws ShareLinkNotUsableException
     */
    public function addChildToTrainerByCode(PlayerProfile $child, string $code): PlayerTrainerMembership
    {
        $trainerId = $this->publicTenantCodes->findTrainerIdByCode($code);

        if (null === $trainerId) {
            throw ShareLinkNotUsableException::unknown();
        }

        $trainer = $this->trainers->find($trainerId);

        if (null === $trainer) {
            throw ShareLinkNotUsableException::unknown();
        }

        $this->tenantContext->activateFor($trainer);
        $shareLink = $this->shareLinks->findOneByCode($code);

        if (null === $shareLink) {
            throw ShareLinkNotUsableException::unknown();
        }

        $this->guardUsable($shareLink);

        $membership = $this->membershipService->associatePlayer($trainer, $child, PlayerTrainerMembership::SOURCE_SHARELINK, $shareLink);
        $this->shareLinkService->recordUse($shareLink);

        return $membership;
    }

    /**
     * AC-01-24: removes a child from one trainer. The confirmation warning
     * about cancelling upcoming RSVPs is a UI-layer concern (rendered before
     * this POST is even reachable, per api-designer-spec) — this method only
     * performs the soft removal itself.
     */
    public function removeChildFromTrainer(PlayerTrainerMembership $membership): void
    {
        $this->membershipService->removePlayerFromTrainer($membership);
    }

    /**
     * AC-01-27/28: the per-child token-approval toggle, changeable at any
     * time from the child's profile settings.
     */
    public function setTokenApprovalBypass(ParentChildLink $link, bool $allowed): void
    {
        $link->setAllowTokenSpendingWithoutApproval($allowed);
    }

    private function guardUsable(ShareLink $shareLink): void
    {
        $now = new \DateTimeImmutable();

        if ($shareLink->isRevoked()) {
            throw ShareLinkNotUsableException::revoked();
        }

        if ($shareLink->isExpired($now)) {
            throw ShareLinkNotUsableException::expired();
        }

        if ($shareLink->isExhausted()) {
            throw ShareLinkNotUsableException::exhausted();
        }
    }
}
