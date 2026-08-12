<?php

declare(strict_types=1);

namespace App\Content\Controller;

use App\Content\Dto\VideoItemInput;
use App\Content\Entity\Drill;
use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAssignment;
use App\Content\Entity\PlaylistItem;
use App\Content\Form\AddDrillToPlaylistType;
use App\Content\Form\AssignPlaylistType;
use App\Content\Form\LearnPlaylistType;
use App\Content\Form\PlaylistEditType;
use App\Content\Form\PlaylistVisibilityType;
use App\Content\Form\PracticePlaylistType;
use App\Content\Form\VideoItemType;
use App\Content\Repository\DrillRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Repository\PlaylistRepository;
use App\Content\Service\PlaylistAssignmentService;
use App\Content\Service\PlaylistService;
use App\Content\Voter\PlaylistVoter;
use App\Crm\Entity\Label;
use App\Crm\Repository\LabelRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-04.01/04/05/06/09/10: the trainer's Learn/Practice tabs — create, view,
 * edit, reorder, publish-toggle, assign, and delete playlists; add drills to
 * a Practice playlist by reference.
 *
 * @see specs/api-designer-spec.md "Content module" — trainer console table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerPlaylistController extends AbstractController
{
    public function __construct(
        private readonly PlaylistRepository $playlists,
        private readonly PlaylistItemRepository $playlistItems,
        private readonly DrillRepository $drills,
        private readonly PlaylistService $playlistService,
        private readonly PlaylistAssignmentService $assignmentService,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly LabelRepository $labels,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * US-04.01 narrative.
     */
    #[Route('/trainer/content/learn', name: 'content_trainer_learn_index', methods: ['GET'])]
    public function learnIndex(): Response
    {
        return $this->render('content/trainer_playlist_index.html.twig', [
            'pillar' => Playlist::PILLAR_LEARN,
            'playlists' => $this->playlists->findAllForActiveTenant($this->currentTrainer(), Playlist::PILLAR_LEARN),
        ]);
    }

    /**
     * AC-04-1..3.
     */
    #[Route('/trainer/content/learn/new', name: 'content_trainer_learn_create', methods: ['GET', 'POST'])]
    public function learnCreate(Request $request): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_CREATE);

        $form = $this->createForm(LearnPlaylistType::class, $this->defaultLearnData());
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            $playlist = $this->playlistService->createLearnPlaylist(
                $this->currentTrainer(),
                (string) $data['title'],
                $this->blankToNull($data['description'] ?? null),
                $this->selectedList($data['filterSkillLevels'] ?? null),
                $this->parseCommaList($data['filterPositions'] ?? null),
                $this->parseCommaList($data['filterAgeLevels'] ?? null),
                (bool) ($data['isPublic'] ?? false),
                (string) $data['audience'],
                $this->toVideoItemInputs($data['videos'] ?? []),
            );

            $this->addFlash('success', 'Playlist created.');

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_learn_form.html.twig', ['form' => $form]);
    }

    /**
     * US-04.04 narrative.
     */
    #[Route('/trainer/content/practice', name: 'content_trainer_practice_index', methods: ['GET'])]
    public function practiceIndex(): Response
    {
        return $this->render('content/trainer_playlist_index.html.twig', [
            'pillar' => Playlist::PILLAR_PRACTICE,
            'playlists' => $this->playlists->findAllForActiveTenant($this->currentTrainer(), Playlist::PILLAR_PRACTICE),
        ]);
    }

    /**
     * AC-04-11/12.
     */
    #[Route('/trainer/content/practice/new', name: 'content_trainer_practice_create', methods: ['GET', 'POST'])]
    public function practiceCreate(Request $request): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_CREATE);

        $form = $this->createForm(PracticePlaylistType::class, $this->defaultPracticeData());
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            $playlist = $this->playlistService->createPracticePlaylist(
                $this->currentTrainer(),
                (string) $data['title'],
                $this->blankToNull($data['description'] ?? null),
                $this->selectedList($data['filterSkillLevels'] ?? null),
                $this->parseCommaList($data['filterPositions'] ?? null),
                $this->parseCommaList($data['filterAgeLevels'] ?? null),
                (bool) ($data['isPublic'] ?? false),
                (string) $data['audience'],
            );

            $this->addFlash('success', 'Playlist created. Add drills from the Drill Database below.');

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_practice_form.html.twig', ['form' => $form]);
    }

    /**
     * US-04.09 narrative.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}', name: 'content_trainer_playlist_show', methods: ['GET'])]
    public function show(Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_VIEW, $playlist);

        return $this->render('content/trainer_playlist_show.html.twig', [
            'playlist' => $playlist,
            'items' => $this->playlistItems->findForPlaylist($playlist),
            'assignments' => $this->assignmentService->statusCounts($playlist->getTrainer(), $playlist),
            'candidateDrills' => Playlist::PILLAR_PRACTICE === $playlist->getPillar() ? $this->drills->findOwnForActiveTenant($playlist->getTrainer()) : [],
        ]);
    }

    /**
     * AC-04-31..33.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/edit', name: 'content_trainer_playlist_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_EDIT, $playlist);

        $form = $this->createForm(PlaylistEditType::class, [
            'title' => $playlist->getTitle(),
            'description' => $playlist->getDescription(),
            'filterSkillLevels' => $playlist->getFilterSkillLevels() ?? [],
            'filterPositions' => null === $playlist->getFilterPositions() ? null : implode(', ', $playlist->getFilterPositions()),
            'filterAgeLevels' => null === $playlist->getFilterAgeLevels() ? null : implode(', ', $playlist->getFilterAgeLevels()),
            'priceUsdMinorUnits' => $playlist->getPriceUsdMinorUnits(),
            'priceTokens' => $playlist->getPriceTokens(),
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            $this->playlistService->updateDetails(
                $playlist,
                (string) $data['title'],
                $this->blankToNull($data['description'] ?? null),
                $this->selectedList($data['filterSkillLevels'] ?? null),
                $this->parseCommaList($data['filterPositions'] ?? null),
                $this->parseCommaList($data['filterAgeLevels'] ?? null),
                (int) $data['priceUsdMinorUnits'],
                (int) $data['priceTokens'],
            );

            $this->addFlash('success', 'Playlist updated!');

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_playlist_edit.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * AC-04-2/AC-04-12: drag-and-drop reorder — JSON in, 204 out. See
     * "JSON vs. HTML §2" in specs/api-designer-spec.md.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/items/reorder', name: 'content_trainer_playlist_items_reorder', methods: ['POST'])]
    public function itemsReorder(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_EDIT, $playlist);

        /** @var array{order?: list<mixed>} $payload */
        $payload = $request->toArray();
        $orderedIds = array_map('intval', $payload['order'] ?? []);

        $this->playlistService->reorderItems($playlist, $orderedIds);

        return new Response('', Response::HTTP_NO_CONTENT);
    }

    /**
     * Removes one item from the playlist — AC-04-31's "removes videos/drills".
     * Not itself named as a distinct route in specs/api-designer-spec.md's
     * route table (which only names create/reorder/visibility/delete/assign/
     * drill-add explicitly); added because AC-04-31 states removal as a
     * capability with no other reachable path, mirroring the established
     * `scheduling_trainer_event_rsvp_remove` shape (POST + CSRF, redirect
     * back with flash). Recorded in the coder's final report.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/items/{item<\d+>}/remove', name: 'content_trainer_playlist_item_remove', methods: ['POST'])]
    public function itemRemove(Request $request, Playlist $playlist, PlaylistItem $item): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_EDIT, $playlist);

        if ($item->getPlaylist()->getId() !== $playlist->getId()) {
            throw $this->createNotFoundException();
        }

        if (!$this->isCsrfTokenValid('playlist-item-remove'.$item->getId(), $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }

        $this->playlistService->removeItem($item);
        $this->addFlash('success', 'Item removed.');

        return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
    }

    /**
     * AC-04-2: one more inline video appended to a Learn playlist.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/videos', name: 'content_trainer_playlist_video_add', methods: ['GET', 'POST'])]
    public function videoAdd(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_EDIT, $playlist);

        $form = $this->createForm(VideoItemType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{youtubeUrl: string, title: string, instructions: ?string, tags: ?string, durationSeconds: ?int} $data */
            $data = $form->getData();

            $this->playlistService->addVideoItem($playlist, new VideoItemInput(
                $data['youtubeUrl'],
                $data['title'],
                $this->blankToNull($data['instructions']),
                $this->parseCommaList($data['tags']),
                $data['durationSeconds'],
            ));

            $this->addFlash('success', 'Video added.');

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_playlist_video_add.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * AC-04-17/18/41, A9.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/visibility', name: 'content_trainer_playlist_visibility', methods: ['GET', 'POST'])]
    public function visibility(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_PUBLISH_TOGGLE, $playlist);

        $form = $this->createForm(PlaylistVisibilityType::class, ['publication' => $playlist->isPublic(), 'audience' => $playlist->getAudience()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{publication: bool, audience: string} $data */
            $data = $form->getData();

            $this->playlistService->setVisibility($playlist, $data['publication'], $data['audience']);
            $this->addFlash('success', $data['publication'] ? 'Playlist is now public.' : 'Playlist is now private.');

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_playlist_visibility.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * AC-04-34, AC-04-36.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/delete', name: 'content_trainer_playlist_delete', methods: ['GET', 'POST'])]
    public function delete(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_DELETE, $playlist);

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('playlist-delete'.$playlist->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            $this->playlistService->delete($playlist);
            $this->addFlash('success', sprintf('"%s" deleted.', $playlist->getTitle()));

            return $this->redirectToRoute(Playlist::PILLAR_LEARN === $playlist->getPillar() ? 'content_trainer_learn_index' : 'content_trainer_practice_index');
        }

        return $this->render('content/trainer_playlist_delete.html.twig', ['playlist' => $playlist]);
    }

    /**
     * AC-04-13..16, BR-04-13..15.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/assign', name: 'content_trainer_playlist_assign', methods: ['GET', 'POST'])]
    public function assign(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_ASSIGN, $playlist);

        $trainer = $playlist->getTrainer();
        $form = $this->createForm(AssignPlaylistType::class, null, [
            'candidatePlayers' => $this->candidatePlayers($trainer),
            'candidateLabels' => $this->labels->findAllForActiveTenant(),
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{targetType: string, targetPlayers: iterable<PlayerProfile>, targetLabel: ?Label, targetSkillLevel: ?string, dueDate: ?\DateTimeImmutable, note: ?string} $data */
            $data = $form->getData();
            $count = $this->createAssignments($trainer, $playlist, $data);

            $this->addFlash('success', sprintf('Playlist assigned (%d selection%s).', $count, 1 === $count ? '' : 's'));

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_playlist_assign.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * AC-04-8/AC-04-12, BR-04-12: adds an existing drill (own or public) to
     * a Practice playlist by reference.
     */
    #[Route('/trainer/content/playlists/{playlist<\d+>}/drills', name: 'content_trainer_playlist_drill_add', methods: ['GET', 'POST'])]
    public function drillAdd(Request $request, Playlist $playlist): Response
    {
        $this->denyAccessUnlessGranted(PlaylistVoter::PLAYLIST_EDIT, $playlist);

        $trainer = $playlist->getTrainer();
        $candidates = [...$this->drills->findOwnForActiveTenant($trainer), ...$this->drills->searchPublic($trainer, null, null, null, null, null)];

        $form = $this->createForm(AddDrillToPlaylistType::class, null, ['candidates' => $candidates]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{drill: Drill, trainerNotes: ?string} $data */
            $data = $form->getData();

            $this->playlistService->addExistingItem($playlist, $data['drill'], true, $this->blankToNull($data['trainerNotes']));
            $this->addFlash('success', sprintf('Drill added to %s', $playlist->getTitle()));

            return $this->redirectToRoute('content_trainer_playlist_show', ['playlist' => $playlist->getId()]);
        }

        return $this->render('content/trainer_playlist_drill_add.html.twig', ['form' => $form, 'playlist' => $playlist]);
    }

    /**
     * @param array{targetType: string, targetPlayers: iterable<PlayerProfile>, targetLabel: ?Label, targetSkillLevel: ?string, dueDate: ?\DateTimeImmutable, note: ?string} $data
     */
    private function createAssignments(Trainer $trainer, Playlist $playlist, array $data): int
    {
        $note = $this->blankToNull($data['note'] ?? null);

        if (PlaylistAssignment::TARGET_PLAYER === $data['targetType']) {
            $players = $data['targetPlayers'] instanceof \Traversable ? iterator_to_array($data['targetPlayers'], false) : $data['targetPlayers'];
            foreach ($players as $player) {
                $this->assignmentService->assign($trainer, $playlist, $this->actor(), PlaylistAssignment::TARGET_PLAYER, $player, null, null, $data['dueDate'], $note);
            }

            return \count($players);
        }

        if (PlaylistAssignment::TARGET_LABEL === $data['targetType']) {
            $this->assignmentService->assign($trainer, $playlist, $this->actor(), PlaylistAssignment::TARGET_LABEL, null, $data['targetLabel'], null, $data['dueDate'], $note);

            return 1;
        }

        $this->assignmentService->assign($trainer, $playlist, $this->actor(), PlaylistAssignment::TARGET_SKILL_LEVEL, null, null, $data['targetSkillLevel'], $data['dueDate'], $note);

        return 1;
    }

    /**
     * @return array<int, PlayerProfile>
     */
    private function candidatePlayers(Trainer $trainer): array
    {
        $candidates = [];
        foreach ($this->memberships->findActiveForActiveTenant() as $membership) {
            $candidates[(int) $membership->getPlayer()->getId()] = $membership->getPlayer();
        }

        return $candidates;
    }

    /**
     * @param iterable<mixed> $rows CollectionType submits an array keyed by
     *                              index; each entry SHOULD be an array
     *                              matching VideoItemType's fields, but is
     *                              typed loosely here since it crosses an
     *                              HTTP form boundary.
     *
     * @return list<VideoItemInput>
     */
    private function toVideoItemInputs(iterable $rows): array
    {
        $videos = [];

        foreach ($rows as $row) {
            if (!\is_array($row)) {
                continue;
            }

            $videos[] = new VideoItemInput(
                (string) ($row['youtubeUrl'] ?? ''),
                (string) ($row['title'] ?? ''),
                $this->blankToNull($row['instructions'] ?? null),
                $this->parseCommaList($row['tags'] ?? null),
                isset($row['durationSeconds']) && '' !== $row['durationSeconds'] ? (int) $row['durationSeconds'] : null,
            );
        }

        return $videos;
    }

    /**
     * @return array<string, mixed>
     */
    private function defaultLearnData(): array
    {
        return ['isPublic' => false, 'audience' => Playlist::AUDIENCE_PLAYERS_AND_COACHES, 'videos' => [[]]];
    }

    /**
     * @return array<string, mixed>
     */
    private function defaultPracticeData(): array
    {
        return ['isPublic' => false, 'audience' => Playlist::AUDIENCE_PLAYERS_AND_COACHES];
    }

    /**
     * A `multiple` ChoiceType submits a list, and an empty selection means
     * "no filter" — stored as null, matching what the comma-separated field
     * produced for an empty string.
     *
     * @return list<string>|null
     */
    private function selectedList(mixed $raw): ?array
    {
        if (!\is_array($raw) || [] === $raw) {
            return null;
        }

        return array_values(array_map(strval(...), $raw));
    }

    /**
     * @return list<string>|null
     */
    private function parseCommaList(mixed $raw): ?array
    {
        if (!\is_string($raw) || '' === trim($raw)) {
            return null;
        }

        $items = array_values(array_filter(array_map('trim', explode(',', $raw)), static fn (string $v): bool => '' !== $v));

        return [] === $items ? null : $items;
    }

    private function blankToNull(mixed $value): ?string
    {
        return \is_string($value) && '' !== trim($value) ? $value : null;
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
