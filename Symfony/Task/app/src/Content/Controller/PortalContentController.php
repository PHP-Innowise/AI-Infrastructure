<?php

declare(strict_types=1);

namespace App\Content\Controller;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Playlist;
use App\Content\Form\PurchaseMethodType;
use App\Content\Repository\ContentProgressRepository;
use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Content\Service\ContentAssignmentResolver;
use App\Content\Service\ContentProgressService;
use App\Content\Service\PurchasePlaylistAccessService;
use App\Content\Voter\ContentItemVoter;
use App\Content\Voter\PlaylistVoter;
use App\Growth\Exception\InvalidCouponException;
use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Form\ApprovalDecisionType;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\ChildApprovalService;
use App\Identity\Service\PlayerContextResolver;
use App\Identity\Voter\ChildApprovalVoter;
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
 * US-04.07/08: the player-facing LPPP portal — Learn library, Practice
 * workouts, the video/drill player page (with auto-completion), and the
 * Progress dashboard. AC-04-26 (a parent switching between children) needs
 * no code here at all: `PlayerContextResolver::resolve()` already reflects
 * whichever child is currently selected via the existing
 * `identity_portal_context_child_switch` mechanism (Epic-01), reused as-is.
 *
 * @see specs/api-designer-spec.md "Content module" — player portal table
 */
#[IsGranted('ROLE_PLAYER')]
final class PortalContentController extends AbstractController
{
    public function __construct(
        private readonly PlaylistRepository $playlists,
        private readonly PlaylistItemRepository $playlistItems,
        private readonly PlaylistAccessGrantRepository $accessGrants,
        private readonly ContentProgressRepository $progressRepository,
        private readonly ContentAssignmentResolver $assignmentResolver,
        private readonly ContentProgressService $progressService,
        private readonly PurchasePlaylistAccessService $purchaseService,
        private readonly PlayerContextResolver $playerContext,
        private readonly ChildApprovalService $childApprovalService,
        private readonly PlayerProfileRepository $playerProfiles,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-04-20/21/23: `tab` defaults to `learn`. `tab=progress` sends the
     * player to the dedicated Progress dashboard route rather than
     * duplicating its rendering here.
     */
    #[Route('/portal/content', name: 'content_portal_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $tab = (string) $request->query->get('tab', 'learn');

        if ('progress' === $tab) {
            return $this->redirectToRoute('content_portal_progress');
        }

        $trainer = $this->currentTrainer();
        $player = $this->currentPlayer($request);
        $pillar = 'practice' === $tab ? Playlist::PILLAR_PRACTICE : Playlist::PILLAR_LEARN;

        if (Playlist::PILLAR_PRACTICE === $pillar) {
            // AC-04-23: the Practice tab is assignment-driven, not the
            // general library.
            $playlistRows = $this->assignmentResolver->assignedPlaylistsForPlayer($trainer, $player, Playlist::PILLAR_PRACTICE);

            return $this->render('content/portal_index.html.twig', [
                'tab' => 'practice',
                'rows' => array_map(fn (Playlist $p): array => $this->practiceRow($trainer, $player, $p), $playlistRows),
            ]);
        }

        // AC-04-21: the Learn tab is the broader Content Library — every
        // player-visible Learn playlist in this trainer context, purchased
        // or not, with a "Suggested" badge for assigned/suggested ones.
        $library = $this->playlists->findPlayerLibraryForActiveTenant($trainer, Playlist::PILLAR_LEARN);

        return $this->render('content/portal_index.html.twig', [
            'tab' => 'learn',
            'rows' => array_map(fn (Playlist $p): array => $this->learnRow($trainer, $player, $p), $library),
        ]);
    }

    /**
     * AC-04-21/22/23: locked (price + purchase CTA) or unlocked (item list
     * with completion checkmarks).
     */
    #[Route('/portal/content/playlists/{playlist<\d+>}', name: 'content_portal_playlist_show', methods: ['GET'])]
    public function playlistShow(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_VIEW, $playlist);

        $player = $this->currentPlayer($request);
        $grant = $this->accessGrants->findOneByPlaylistAndPlayer($playlist, $player);
        $items = $this->playlistItems->findForPlaylist($playlist);

        return $this->render('content/portal_playlist_show.html.twig', [
            'playlist' => $playlist,
            'items' => $items,
            'locked' => null === $grant,
            'progress' => null === $grant ? null : $this->progressService->playlistProgress($playlist->getTrainer(), $player, $playlist),
            'completedItemIds' => $this->completedItemIds($playlist->getTrainer(), $player),
        ]);
    }

    /**
     * AC-05-18/19, BR-04-6..9: token unlocks inline; card 303s to Stripe
     * Checkout in this SAME request (specs/api-designer-spec.md "Billing
     * module": "there is no separate 'create checkout session' endpoint").
     */
    #[Route('/portal/content/playlists/{playlist<\d+>}/checkout', name: 'content_portal_playlist_checkout', methods: ['GET', 'POST'])]
    public function checkout(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_PURCHASE, $playlist);

        $player = $this->currentPlayer($request);
        $form = $this->createForm(PurchaseMethodType::class, ['method' => PurchasePlaylistAccessService::METHOD_TOKEN]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{method: string, couponCode: ?string} $data */
            $data = $form->getData();

            try {
                $outcome = $this->purchaseService->purchase($playlist, $player, $this->actor(), $data['method'], $data['couponCode']);
            } catch (InvalidCouponException $e) {
                // AC-06-23: "Invalid or expired code" — no discount
                // applied, no purchase attempted.
                $form->get('couponCode')->addError(new FormError($e->getMessage()));

                return $this->render('content/portal_playlist_checkout.html.twig', ['form' => $form, 'playlist' => $playlist]);
            }

            if (null !== $outcome->grant) {
                $this->addFlash('success', sprintf('%s unlocked!', $playlist->getTitle()));

                return $this->redirectToRoute('content_portal_playlist_show', ['playlist' => $playlist->getId()]);
            }

            if (null !== $outcome->redirectUrl) {
                return $this->redirect($outcome->redirectUrl);
            }

            if ($outcome->failed) {
                $this->addFlash('error', 'Your purchase could not be completed. Please try again.');

                return $this->redirectToRoute('content_portal_playlist_show', ['playlist' => $playlist->getId()]);
            }

            $this->addFlash('info', 'Your purchase request has been submitted and is pending.');

            return $this->redirectToRoute('content_portal_index', ['tab' => Playlist::PILLAR_LEARN === $playlist->getPillar() ? 'learn' : 'practice']);
        }

        return $this->render('content/portal_playlist_checkout.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * AC-04-24: embedded YouTube player, title, controls, and the Text
     * Instructions section.
     */
    #[Route('/portal/content/items/{item<\d+>}/play', name: 'content_portal_item_play', methods: ['GET'])]
    public function itemPlay(Request $request, ContentItem $item): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_VIEW, $item);

        return $this->render('content/portal_item_play.html.twig', [
            'item' => $item,
            'next' => $this->nextItemInPlaylist($request, $item),
        ]);
    }

    /**
     * AC-04-25: "a 'Next' control to advance to the next video/drill in the
     * playlist" — resolved only when reached WITH a `?playlist=` context
     * (one content item can belong to several playlists, per BR-04-12
     * reuse, so "the" playlist is ambiguous without it).
     */
    private function nextItemInPlaylist(Request $request, ContentItem $item): ?ContentItem
    {
        $playlistId = $request->query->getInt('playlist');

        if (0 === $playlistId) {
            return null;
        }

        $playlist = $this->playlists->find($playlistId);

        if (null === $playlist) {
            return null;
        }

        $items = $this->playlistItems->findForPlaylist($playlist);
        $foundCurrent = false;

        foreach ($items as $playlistItem) {
            if ($foundCurrent) {
                return $playlistItem->getContentItem();
            }

            if ($playlistItem->getContentItem()->getId() === $item->getId()) {
                $foundCurrent = true;
            }
        }

        return null;
    }

    /**
     * AC-04-25, BR-04-16: fired from the YouTube IFrame API's onStateChange
     * (PLAYING) — a widget event, not a page navigation. JSON in (empty
     * body), JSON out, per "JSON vs. HTML §2".
     */
    #[Route('/portal/content/items/{item<\d+>}/complete', name: 'content_portal_item_complete', methods: ['POST'])]
    public function itemComplete(Request $request, ContentItem $item): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_VIEW, $item);

        $player = $this->currentPlayer($request);
        $this->progressService->recordEngagementStarted($this->currentTrainer(), $player, $item);

        return $this->json(['completed' => true]);
    }

    /**
     * AC-04-27..29: overall stats, recent activity, incomplete assignments,
     * per-playlist progress bars.
     */
    #[Route('/portal/content/progress', name: 'content_portal_progress', methods: ['GET'])]
    public function progress(Request $request): Response
    {
        $trainer = $this->currentTrainer();
        $player = $this->currentPlayer($request);

        $assignedLearn = $this->assignmentResolver->assignedPlaylistsForPlayer($trainer, $player, Playlist::PILLAR_LEARN);
        $assignedPractice = $this->assignmentResolver->assignedPlaylistsForPlayer($trainer, $player, Playlist::PILLAR_PRACTICE);
        $assigned = [...$assignedLearn, ...$assignedPractice];

        $incomplete = $this->progressService->incompleteAssignedPlaylists($trainer, $player, $assigned);

        return $this->render('content/portal_progress.html.twig', [
            'stats' => $this->progressService->overallStats($trainer, $player, $assigned),
            'recentActivity' => $this->progressService->recentActivity($trainer, $player),
            'incomplete' => array_map(fn (Playlist $p): array => [
                'playlist' => $p,
                'progress' => $this->progressService->playlistProgress($trainer, $player, $p),
                'dueDate' => $this->assignmentResolver->nearestDueDateForPlayer($trainer, $p, $player),
            ], $incomplete),
            'assigned' => array_map(fn (Playlist $p): array => [
                'playlist' => $p,
                'progress' => $this->progressService->playlistProgress($trainer, $player, $p),
            ], $assigned),
        ]);
    }

    /**
     * @return array{playlist: Playlist, locked: bool, suggested: bool, progress: ?array{completed: int, total: int}}
     */
    private function learnRow(Trainer $trainer, PlayerProfile $player, Playlist $playlist): array
    {
        $unlocked = null !== $this->accessGrants->findOneByPlaylistAndPlayer($playlist, $player);

        return [
            'playlist' => $playlist,
            'locked' => !$unlocked,
            'suggested' => $this->assignmentResolver->isAssignedToPlayer($trainer, $playlist, $player),
            'progress' => $unlocked ? $this->progressService->playlistProgress($trainer, $player, $playlist) : null,
        ];
    }

    /**
     * @return array{playlist: Playlist, dueDate: ?\DateTimeImmutable, progress: array{completed: int, total: int}}
     */
    private function practiceRow(Trainer $trainer, PlayerProfile $player, Playlist $playlist): array
    {
        return [
            'playlist' => $playlist,
            'dueDate' => $this->assignmentResolver->nearestDueDateForPlayer($trainer, $playlist, $player),
            'progress' => $this->progressService->playlistProgress($trainer, $player, $playlist),
        ];
    }

    /**
     * AC-04-22/23: every content item this player has completed, within
     * this tenant — a direct read of `ContentProgress`, not derived from the
     * playlist-level count `playlistProgress()` returns. The template
     * indexes into this by each row's own content-item id.
     *
     * @return array<int, true>
     */
    private function completedItemIds(Trainer $trainer, PlayerProfile $player): array
    {
        $completed = [];

        foreach ($this->progressRepository->findForPlayer($trainer, $player) as $row) {
            if ($row->isCompleted()) {
                $completed[(int) $row->getContentItem()->getId()] = true;
            }
        }

        return $completed;
    }

    /**
     * AC-01-26-style decision on a pending ACTION_CONTENT_PURCHASE request —
     * distinct from `App\Identity\Controller\ApprovalController`'s own
     * generic decide routes because Identity must never call into Content
     * directly (module dependency direction), mirroring
     * `PortalReservationController::decideApproval()`'s own precedent
     * exactly.
     */
    #[Route('/portal/content/purchase-approvals/{approval<\d+>}/approve', name: 'content_portal_purchase_approval_approve', methods: ['GET', 'POST'])]
    public function purchaseApprovalApprove(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decidePurchaseApproval($request, $approval, true);
    }

    #[Route('/portal/content/purchase-approvals/{approval<\d+>}/deny', name: 'content_portal_purchase_approval_deny', methods: ['GET', 'POST'])]
    public function purchaseApprovalDeny(Request $request, ChildApprovalRequest $approval): Response
    {
        return $this->decidePurchaseApproval($request, $approval, false);
    }

    private function decidePurchaseApproval(Request $request, ChildApprovalRequest $approval, bool $approving): Response
    {
        $this->denyAccessUnlessGranted(ChildApprovalVoter::CHILD_APPROVAL_DECIDE, $approval);

        if (ChildApprovalRequest::ACTION_CONTENT_PURCHASE !== $approval->getActionType()) {
            throw $this->createNotFoundException('Not a content-purchase approval request.');
        }

        $form = $this->createForm(ApprovalDecisionType::class, null, ['label' => $approving ? 'Approve' : 'Deny']);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{note: ?string} $data */
            $data = $form->getData();

            if ($approving) {
                $this->childApprovalService->approve($approval, $data['note']);
                $this->completePurchaseAfterApproval($approval);
            } else {
                $this->childApprovalService->deny($approval, $data['note']);
            }

            $this->addFlash('success', $approving ? 'Request approved.' : 'Request denied.');

            return $this->redirectToRoute('identity_portal_approvals_index');
        }

        return $this->render('identity/approval_decide.html.twig', [
            'form' => $form,
            'approval' => $approval,
            'approving' => $approving,
        ]);
    }

    private function completePurchaseAfterApproval(ChildApprovalRequest $approval): void
    {
        $playlistId = $approval->getRequestedPlaylistId();

        if (null === $playlistId) {
            return;
        }

        $playlist = $this->playlists->find($playlistId);

        if (null === $playlist || $playlist->getTrainer()->getId() !== $approval->getTrainer()->getId()) {
            return;
        }

        $child = $this->playerProfiles->find($approval->getChildPlayer()->getId());

        if (null === $child) {
            return;
        }

        $this->purchaseService->completeAfterParentApproval($playlist, $child, $approval->getParentAccount());
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
