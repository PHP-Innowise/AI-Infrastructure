<?php

declare(strict_types=1);

namespace App\Content\Voter;

use App\Content\Entity\Playlist;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Tenancy\TenantContext;
use Symfony\Component\HttpFoundation\RequestStack;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-04-3..5/9/12..15/17, A8, A9, AC-04-1..23, AC-04-31..36, AC-04-41.
 *
 * **The publication exception is a read-predicate widening, never a write
 * one.** `PLAYLIST_VIEW` grants cross-tenant for anything ever published;
 * `PLAYLIST_EDIT`/`PLAYLIST_DELETE`/`PLAYLIST_PUBLISH_TOGGLE`/
 * `PLAYLIST_ASSIGN` never do, regardless of publication state — ownership of
 * a public item never transfers, only its readability widens
 * (specs/security-voter-designer-design.md "Content module").
 *
 * Every "is this MY tenant's row" check here compares against
 * `TenantContext`, NOT against structural invisibility the way
 * `EventVoter`/`LabelVoter` can (their comment: "structurally same-tenant
 * already: RLS + the Doctrine filter make a foreign-tenant row invisible
 * before this voter runs"). That shortcut does not hold for `Playlist` —
 * see its own docblock for why it carries no `#[TrainerScoped]` filter —
 * so ownership is re-verified explicitly on every mutating attribute here.
 *
 * No `ROLE_SUPER_ADMIN` clause on any attribute: no route edits an
 * individual trainer's playlist as Super Admin, and
 * `content_super_admin_analytics` reads through `CrossTenantReadService`,
 * never through this voter (security-voter-designer-design.md "Content
 * module": "a natural absence, not an unresolved mechanism").
 *
 * `FeatureGate::isEnabled($trainer, 'lppp')` is NOT checked here — see the
 * coder's final report: `FeatureToggle`/`FeatureGate` (Administration/
 * Epic-07) do not exist in this codebase yet.
 *
 * @see specs/security-voter-designer-design.md "Content module"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md
 */
/**
 * @extends Voter<string, Playlist|null>
 */
final class PlaylistVoter extends Voter
{
    public const PLAYLIST_CREATE = 'PLAYLIST_CREATE';
    public const PLAYLIST_VIEW = 'PLAYLIST_VIEW';
    public const PLAYLIST_EDIT = 'PLAYLIST_EDIT';
    public const PLAYLIST_PUBLISH_TOGGLE = 'PLAYLIST_PUBLISH_TOGGLE';
    public const PLAYLIST_DELETE = 'PLAYLIST_DELETE';
    public const PLAYLIST_ASSIGN = 'PLAYLIST_ASSIGN';
    public const PLAYLIST_PURCHASE = 'PLAYLIST_PURCHASE';

    /**
     * @var list<string>
     */
    private const OWNER_ONLY_ATTRIBUTES = [self::PLAYLIST_EDIT, self::PLAYLIST_PUBLISH_TOGGLE, self::PLAYLIST_DELETE, self::PLAYLIST_ASSIGN];

    public function __construct(
        private readonly TenantContext $tenantContext,
        private readonly RequestStack $requestStack,
        private readonly PlayerContextResolver $playerContext,
        private readonly PlayerTrainerMembershipRepository $memberships,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (self::PLAYLIST_CREATE === $attribute) {
            return null === $subject;
        }

        return \in_array($attribute, [
            self::PLAYLIST_VIEW, self::PLAYLIST_EDIT, self::PLAYLIST_PUBLISH_TOGGLE,
            self::PLAYLIST_DELETE, self::PLAYLIST_ASSIGN, self::PLAYLIST_PURCHASE,
        ], true) && $subject instanceof Playlist;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::PLAYLIST_CREATE === $attribute) {
            return AccountRole::Trainer === $actor->getRole();
        }

        \assert($subject instanceof Playlist);

        if (self::PLAYLIST_VIEW === $attribute) {
            return $this->voteView($subject, $actor);
        }

        if (self::PLAYLIST_PURCHASE === $attribute) {
            return $this->votePurchase($subject, $actor);
        }

        if (\in_array($attribute, self::OWNER_ONLY_ATTRIBUTES, true)) {
            // Never widened by publication — BR-04-11, ownership never
            // transfers. Deletion/edit/assignment/publish-toggle are
            // trainer-only, own-tenant-only, full stop.
            return AccountRole::Trainer === $actor->getRole() && $this->isOwnTenant($subject);
        }

        return false;
    }

    /**
     * AC-04-8/AC-04-17..19, BR-04-4/5: a trainer may view their OWN
     * playlist (any state) or ANY playlist that has ever been published —
     * the widened read. A coach may view their own trainer's playlists,
     * including coaches-only ones, but never another trainer's (coaches
     * have no "discover public content" capability in this epic). A player
     * may view their CURRENT trainer's playlists, excluding coaches-only
     * ones (AC-04-41) — never a cross-tenant playlist, even a published one:
     * BR-04-10's separated views mean a player's own library stays
     * per-trainer regardless of what publication would otherwise permit a
     * TRAINER to browse.
     */
    private function voteView(Playlist $playlist, Account $actor): bool
    {
        if (AccountRole::Trainer === $actor->getRole()) {
            return $this->isOwnTenant($playlist) || $playlist->hasEverBeenPublished();
        }

        if (AccountRole::Coach === $actor->getRole()) {
            return $this->isOwnTenant($playlist);
        }

        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        if (!$this->isOwnTenant($playlist) || $playlist->isCoachesOnly()) {
            return false;
        }

        return $this->hasActiveMembership($playlist, $actor);
    }

    /**
     * AC-04-22/BR-04-6/7: only a player may attempt a purchase, only for
     * their own current trainer's playlist, only when it is player-visible
     * at all (never coaches-only). Whether a CHILD's specific attempt
     * executes now or waits for a parent is a separate decision
     * (`ChildApprovalVoter::CHILD_APPROVAL_BYPASS`), made by
     * `PurchasePlaylistAccessService`, not this attribute — mirroring
     * `RsvpVoter::RSVP_CREATE` / `RsvpService::rsvp()`'s own split exactly.
     */
    private function votePurchase(Playlist $playlist, Account $actor): bool
    {
        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        if (!$this->isOwnTenant($playlist) || $playlist->isCoachesOnly()) {
            return false;
        }

        return $this->hasActiveMembership($playlist, $actor);
    }

    private function isOwnTenant(Playlist $playlist): bool
    {
        return $playlist->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
    }

    private function hasActiveMembership(Playlist $playlist, Account $actor): bool
    {
        $request = $this->requestStack->getCurrentRequest();

        if (null === $request) {
            return false;
        }

        $player = $this->playerContext->resolve($request, $actor);
        $membership = $this->memberships->findOneByTrainerAndPlayer($playlist->getTrainer(), $player);

        return null !== $membership && $membership->isActive();
    }
}
