<?php

declare(strict_types=1);

namespace App\Content\Voter;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\PlayerContextResolver;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\HttpFoundation\RequestStack;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-04-1/2/18/19, AC-04-4..8, AC-04-24/25, AC-04-35.
 *
 * `CONTENT_ITEM_VIEW` is where the paywall actually bites (see
 * `PlaylistVoter`'s own docblock: "`PLAYLIST_VIEW` always grants... it is
 * `CONTENT_ITEM_VIEW` that denies actual playback while the owning
 * `Playlist` is locked and unpurchased"). Because one `ContentItem` can be
 * referenced by several playlists at once (BR-04-12 reuse), "the owning
 * playlist" is resolved as "every playlist in the player's own tenant that
 * references this item" — grant is reachable through ANY of them the player
 * already holds a `PlaylistAccessGrant` for.
 *
 * Same non-widened-write rule as `PlaylistVoter`: `_EDIT`/`_DELETE`/
 * `_PUBLISH_TOGGLE` are trainer-only, own-tenant-only, never granted merely
 * because the item is published.
 *
 * @see specs/security-voter-designer-design.md "Content module"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md
 */
/**
 * @extends Voter<string, ContentItem|null>
 */
final class ContentItemVoter extends Voter
{
    public const CONTENT_ITEM_CREATE = 'CONTENT_ITEM_CREATE';
    public const CONTENT_ITEM_VIEW = 'CONTENT_ITEM_VIEW';
    public const CONTENT_ITEM_EDIT = 'CONTENT_ITEM_EDIT';
    public const CONTENT_ITEM_DELETE = 'CONTENT_ITEM_DELETE';
    public const CONTENT_ITEM_PUBLISH_TOGGLE = 'CONTENT_ITEM_PUBLISH_TOGGLE';

    /**
     * @var list<string>
     */
    private const OWNER_ONLY_ATTRIBUTES = [self::CONTENT_ITEM_EDIT, self::CONTENT_ITEM_DELETE, self::CONTENT_ITEM_PUBLISH_TOGGLE];

    public function __construct(
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
        private readonly RequestStack $requestStack,
        private readonly PlayerContextResolver $playerContext,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly PlaylistItemRepository $playlistItems,
        private readonly PlaylistAccessGrantRepository $accessGrants,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        if (self::CONTENT_ITEM_CREATE === $attribute) {
            return null === $subject;
        }

        return \in_array($attribute, [
            self::CONTENT_ITEM_VIEW, self::CONTENT_ITEM_EDIT, self::CONTENT_ITEM_DELETE, self::CONTENT_ITEM_PUBLISH_TOGGLE,
        ], true) && $subject instanceof ContentItem;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::CONTENT_ITEM_CREATE === $attribute) {
            return AccountRole::Trainer === $actor->getRole();
        }

        \assert($subject instanceof ContentItem);

        if (self::CONTENT_ITEM_VIEW === $attribute) {
            return $this->voteView($subject, $actor);
        }

        if (\in_array($attribute, self::OWNER_ONLY_ATTRIBUTES, true)) {
            return AccountRole::Trainer === $actor->getRole() && $this->isOwnCreation($subject);
        }

        return false;
    }

    /**
     * AC-04-7/8: a trainer may view their own item (any state) or ANY
     * published item (public-drill preview before adding it to a playlist).
     * A coach may view an item reachable within their own tenant (created
     * there, or already reused into one of that tenant's own playlists),
     * never a bare cross-tenant preview. A player must additionally clear
     * the paywall: AC-04-25's player page only opens for content whose
     * containing playlist(s) — within the player's own tenant — include at
     * least one the player already holds a purchase grant for.
     */
    private function voteView(ContentItem $item, Account $actor): bool
    {
        if (AccountRole::Trainer === $actor->getRole()) {
            return $this->isOwnCreation($item) || $this->isReachableWithinTenant($item) || $item->hasEverBeenPublished();
        }

        if (AccountRole::Coach === $actor->getRole()) {
            return $this->isOwnCreation($item) || $this->isReachableWithinTenant($item);
        }

        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        $tenant = $this->currentTenantReference();

        if (null === $tenant) {
            return false;
        }

        $request = $this->requestStack->getCurrentRequest();

        if (null === $request) {
            return false;
        }

        $player = $this->playerContext->resolve($request, $actor);
        $membership = $this->memberships->findOneByTrainerAndPlayer($tenant, $player);

        if (null === $membership || !$membership->isActive()) {
            return false;
        }

        $playlists = $this->playlistItems->findPlaylistsForContentItemInTenant($tenant, $item);

        return $this->isUnlockedForPlayerByAnyPlaylist($playlists, $player);
    }

    /**
     * @param list<Playlist> $playlists
     */
    private function isUnlockedForPlayerByAnyPlaylist(array $playlists, PlayerProfile $player): bool
    {
        foreach ($playlists as $playlist) {
            if ($playlist->isCoachesOnly()) {
                continue;
            }

            if (null !== $this->accessGrants->findOneByPlaylistAndPlayer($playlist, $player)) {
                return true;
            }
        }

        return false;
    }

    private function isOwnCreation(ContentItem $item): bool
    {
        return $item->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
    }

    /**
     * BR-04-12: reused content has no `trainer_id` match at all — reachable
     * only through a `PlaylistItem` reference belonging to this tenant.
     */
    private function isReachableWithinTenant(ContentItem $item): bool
    {
        $tenant = $this->currentTenantReference();

        return null !== $tenant && [] !== $this->playlistItems->findPlaylistsForContentItemInTenant($tenant, $item);
    }

    /**
     * A lazy proxy reference, not a query — same pattern as
     * `TrainerEventController::currentTrainer()` — since this voter only
     * ever needs the tenant's id for downstream `IDENTITY()` comparisons,
     * and a voter runs on every authorization check.
     */
    private function currentTenantReference(): ?Trainer
    {
        $tenantId = $this->tenantContext->getTrainerIdOrNull();

        if (null === $tenantId) {
            return null;
        }

        /** @var Trainer $trainer */
        $trainer = $this->entityManager->getReference(Trainer::class, $tenantId);

        return $trainer;
    }
}
