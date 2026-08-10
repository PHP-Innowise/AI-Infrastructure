<?php

declare(strict_types=1);

namespace App\Content\Controller;

use App\Content\Entity\Drill;
use App\Content\Entity\Playlist;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Voter\ContentItemVoter;
use App\Content\Voter\PlaylistVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * "User Roles Involved": "the Coach/Contractor can view a trainer's content
 * and their assigned players' progress but cannot create content"
 * (requirements-analyst-epic-04-lp-content-spec.md, Problem). A separate,
 * `ROLE_COACH`-gated controller, matching this codebase's own established
 * shape for role-specific read access (`CoachPlayerController` vs.
 * `TrainerPlayerController`) rather than widening the trainer console's own
 * coarse gate — reuses the trainer console's OWN templates (the display
 * need is identical; only the mutating action links, each already voter-
 * guarded, differ per viewer).
 *
 * No dedicated route is named in specs/api-designer-spec.md's Content route
 * table for this — that table lists no coach-specific content route at all,
 * despite the epic's own role description naming the capability. Added here
 * because `PlaylistVoter`/`ContentItemVoter` were already designed with a
 * coach `VIEW` branch (specs/security-voter-designer-design.md never states
 * a route for it either) with no way to reach it otherwise. Recorded in the
 * coder's final report.
 */
#[IsGranted('ROLE_COACH')]
final class CoachContentController extends AbstractController
{
    public function __construct(
        private readonly PlaylistItemRepository $playlistItems,
    ) {
    }

    #[Route('/coach/content/playlists/{playlist}', name: 'content_coach_playlist_show', methods: ['GET'])]
    public function playlistShow(Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_VIEW, $playlist);

        return $this->render('content/trainer_playlist_show.html.twig', [
            'playlist' => $playlist,
            'items' => $this->playlistItems->findForPlaylist($playlist),
            'assignments' => ['assigned' => 0, 'completed' => 0, 'inProgress' => 0],
            'candidateDrills' => [],
        ]);
    }

    #[Route('/coach/content/drills/{drill}', name: 'content_coach_drill_show', methods: ['GET'])]
    public function drillShow(Drill $drill): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_VIEW, $drill);

        return $this->render('content/trainer_drill_show.html.twig', ['drill' => $drill]);
    }
}
