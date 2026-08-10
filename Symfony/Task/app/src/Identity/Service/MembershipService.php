<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Entity\ShareLink;
use App\Identity\Exception\CoachAlreadyActiveElsewhereException;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Entity\AccountTrainerLink;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Tenancy\TenantContext;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;

/**
 * The sole creator of PlayerTrainerMembership and CoachMembership rows
 * (architecture "Cross-cutting services — MembershipService"), and the sole
 * writer of AccountTrainerLink outside trainer provisioning — see that
 * entity's own docblock.
 *
 * **Always explicit about which Trainer it writes to.** Every public method
 * here activates the target trainer's tenant itself
 * (`TenantContext::activateFor()`) rather than trusting whatever the ambient
 * per-request resolver already decided. This is deliberate: the caller is
 * frequently authorizing a write into a trainer that is *not* the request's
 * ambient tenant — a second trainer's ShareLink (AC-01-13/14), a coach
 * accepting an invite from a trainer they hold no session context for yet, a
 * parent picking a trainer from "My Trainers" for a child. `specs/api-designer-spec.md`,
 * "The gap: creating the first membership from an anonymous ShareLink click"
 * names exactly this hole in the ambient resolver's source list and suggests
 * "a narrow, code-authorized scope opened only for the ShareLink's own
 * trainer, only for the duration of MembershipService's one call" as a
 * structurally plausible fix. This class IS that fix for Identity: the
 * caller resolves and authorizes a `Trainer` entity (via a ShareLink code
 * lookup + voter, or via an already-active AccountTrainerLink), and this
 * service activates that trainer's tenant for the duration of its own
 * transaction. Because each of these calls is the terminal write of its
 * request (immediately followed by a redirect), there is no risk of a later
 * step in the same request silently operating under the wrong tenant.
 *
 * @see specs/architect-architecture.md "Cross-cutting services"
 * @see specs/database-designer-schema.md "`account_trainer_link`" — "Written by MembershipService..."
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-10..12, BR-01-11
 */
final readonly class MembershipService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private TenantContext $tenantContext,
        private PlayerTrainerMembershipRepository $playerMemberships,
        private CoachMembershipRepository $coachMemberships,
        private AccountTrainerLinkRepository $accountTrainerLinks,
    ) {
    }

    /**
     * AC-01-11/13/14: associates a player with a trainer, reusing an existing
     * membership row (reactivated) rather than ever inserting a duplicate for
     * the same (trainer, player) pair — BR-01-12.
     */
    public function associatePlayer(
        Trainer $trainer,
        PlayerProfile $player,
        string $source,
        ?ShareLink $shareLink = null,
    ): PlayerTrainerMembership {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $player, $source, $shareLink): PlayerTrainerMembership {
            $this->tenantContext->activateFor($trainer);

            $membership = $this->playerMemberships->findOneByTrainerAndPlayer($trainer, $player);

            if (null === $membership) {
                $membership = new PlayerTrainerMembership($trainer, $player, $source, $shareLink);
                $this->playerMemberships->add($membership);
            } elseif (!$membership->isActive()) {
                $membership->reactivate($source, $shareLink);
            }
            // Already active: BR-01-12, no-op — this IS the "no duplicate
            // association" outcome, not an error.

            $selfAccount = $player->getSelfAccount();

            if (null !== $selfAccount) {
                $this->syncAccountTrainerLink($selfAccount, $trainer, AccountRole::Player->value);
            }

            return $membership;
        });
    }

    /**
     * AC-01-24: soft-removes a child (or any player) from one trainer.
     * History is preserved on the same row; the caller is responsible for the
     * "cancels upcoming RSVPs" fan-out once Scheduling exists (Epic-02) —
     * that side effect is out of Identity's reach in Epic-01.
     */
    public function removePlayerFromTrainer(PlayerTrainerMembership $membership): void
    {
        $this->entityManager->wrapInTransaction(function () use ($membership): void {
            $this->tenantContext->activateFor($membership->getTrainer());
            $membership->remove();

            $player = $membership->getPlayer();
            $selfAccount = $player->getSelfAccount();

            if (null !== $selfAccount) {
                $link = $this->accountTrainerLinks->findOneByAccountAndTrainer($selfAccount, $membership->getTrainer());
                $link?->deactivate();
            }
        });
    }

    /**
     * AC-01-40/41, BR-01-11/15: a coach accepting an invite. `$status`
     * defaults to Active — registering via the invite link is itself the
     * acceptance (no separate confirmation step is described anywhere in the
     * epic for the coach side, unlike the parent-child approval workflow).
     *
     * BR-01-11's cross-tenant exclusivity is a domain refusal, not a voter
     * decision (security-voter-designer-design.md, "Coach-trainer
     * exclusivity... is a domain rule, not a voter decision"): this method
     * attempts the insert and translates the database's own unique-violation
     * — `uniq_coach_membership_active_account`, which reaches across
     * RLS-hidden tenants by construction — into
     * `CoachAlreadyActiveElsewhereException`.
     *
     * @throws CoachAlreadyActiveElsewhereException
     */
    public function acceptCoachInvite(
        Trainer $trainer,
        Account $coachAccount,
        ShareLink $invite,
        string $status = CoachMembership::STATUS_ACTIVE,
    ): CoachMembership {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $coachAccount, $invite, $status): CoachMembership {
            $this->tenantContext->activateFor($trainer);

            $membership = new CoachMembership($trainer, $coachAccount, $status, $invite);
            $this->coachMemberships->add($membership);

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw CoachAlreadyActiveElsewhereException::forAccountId((int) $coachAccount->getId());
            }

            if (CoachMembership::STATUS_ACTIVE === $status) {
                // wrapInTransaction() flushes again once this closure
                // returns, which picks up this persist alongside the
                // membership already flushed above — both commit together.
                $this->syncAccountTrainerLink($coachAccount, $trainer, AccountRole::Coach->value);
            }

            return $membership;
        });
    }

    /**
     * specs/requirements-analyst-epic-02-event-management-spec.md AC-02-8:
     * "the dropdown includes the trainer themselves (trainers can assign
     * themselves as coach)." A trainer's own account never goes through the
     * invite-and-accept flow `acceptCoachInvite()` models, so this is the
     * lazy, idempotent find-or-create Epic-02's coach-assignment dropdown
     * needs: the first time a trainer picks themselves, their own
     * CoachMembership row is created (Active, no ShareLink) if it does not
     * already exist. Purely additive — every existing caller of this class
     * is unaffected.
     */
    public function ensureSelfCoachMembership(Trainer $trainer): CoachMembership
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer): CoachMembership {
            $this->tenantContext->activateFor($trainer);

            $existing = $this->coachMemberships->findOneForAccountInActiveTenant($trainer->getOwnerAccount());

            if (null !== $existing) {
                return $existing;
            }

            $membership = new CoachMembership($trainer, $trainer->getOwnerAccount(), CoachMembership::STATUS_ACTIVE);
            $this->coachMemberships->add($membership);
            $this->entityManager->flush();

            return $membership;
        });
    }

    /**
     * Upserts the resolver's own source (`account_trainer_link`) — the
     * dual-write this schema's own docblock calls out as "worth a dedicated
     * integration test" (`TenancyIsolationTest` covers the RLS half;
     * `MembershipServiceTest` covers this half).
     */
    private function syncAccountTrainerLink(Account $account, Trainer $trainer, string $roleInTenant): void
    {
        $link = $this->accountTrainerLinks->findOneByAccountAndTrainer($account, $trainer);

        if (null === $link) {
            $this->accountTrainerLinks->add(new AccountTrainerLink($account, $trainer, $roleInTenant));

            return;
        }

        if (!$link->isActive()) {
            $link->reactivate();
        }
    }
}
