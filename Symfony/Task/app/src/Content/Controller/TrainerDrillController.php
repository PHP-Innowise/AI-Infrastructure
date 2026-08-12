<?php

declare(strict_types=1);

namespace App\Content\Controller;

use App\Content\Entity\ContentItem;
use App\Content\Entity\Drill;
use App\Content\Form\DrillType;
use App\Content\Form\PublishToggleType;
use App\Content\Repository\DrillRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Content\Service\DrillService;
use App\Content\Voter\ContentItemVoter;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-04.02/03/06/10: the Drill Database — "My Drills", "Public Drills"
 * search/filter/preview, create/edit/delete, and publish-toggle.
 *
 * @see specs/api-designer-spec.md "Content module" — trainer console table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerDrillController extends AbstractController
{
    public function __construct(
        private readonly DrillRepository $drills,
        private readonly PlaylistItemRepository $playlistItems,
        private readonly DrillService $drillService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-04-7/9, BR-04-19: `scope=mine|public` toggles the two lists; the
     * public list supports search + category/difficulty/equipment/duration
     * filters, AND-combined. An empty public result shows the epic's own
     * exact copy ("No drills match your filters. Try adjusting criteria."),
     * rendered by the template from an empty `results` array — nothing
     * special is returned here for the empty case.
     */
    #[Route('/trainer/content/drills', name: 'content_trainer_drills_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $trainer = $this->currentTrainer();
        $scope = 'public' === $request->query->get('scope') ? 'public' : 'mine';

        if ('mine' === $scope) {
            return $this->render('content/trainer_drills_index.html.twig', [
                'scope' => $scope,
                'results' => $this->drills->findOwnForActiveTenant($trainer),
            ]);
        }

        $durationParam = $request->query->get('maxDuration');

        $results = $this->drills->searchPublic(
            $trainer,
            $this->nullableString($request->query->get('q')),
            $this->nullableString($request->query->get('category')),
            $this->nullableString($request->query->get('difficulty')),
            $this->nullableString($request->query->get('equipment')),
            null === $durationParam || '' === $durationParam ? null : (int) $durationParam,
        );

        return $this->render('content/trainer_drills_index.html.twig', ['scope' => $scope, 'results' => $results]);
    }

    /**
     * AC-04-4..6.
     */
    #[Route('/trainer/content/drills/new', name: 'content_trainer_drill_create', methods: ['GET', 'POST'])]
    public function create(Request $request): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_CREATE);

        $form = $this->createForm(DrillType::class, ['difficultyLevel' => Drill::DIFFICULTY_BEGINNER, 'isPublic' => false]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            $drill = $this->drillService->createDrill(
                $this->currentTrainer(),
                (string) $data['title'],
                (string) $data['youtubeUrl'],
                (string) $data['difficultyLevel'],
                $this->parseCommaList($data['categories'] ?? null) ?? [],
                $this->blankToNull($data['instructions'] ?? null),
                $this->parseCommaList($data['tags'] ?? null),
                $this->parseCommaList($data['equipment'] ?? null),
                $this->blankToNull($data['spaceRequirement'] ?? null),
                $this->blankToNull($data['playerCount'] ?? null),
                '' === ($data['durationMinMinutes'] ?? '') ? null : (int) $data['durationMinMinutes'],
                '' === ($data['durationMaxMinutes'] ?? '') ? null : (int) $data['durationMaxMinutes'],
                (bool) ($data['isPublic'] ?? false),
            );

            $this->addFlash('success', 'Drill created.');

            return $this->redirectToRoute('content_trainer_drill_show', ['drill' => $drill->getId()]);
        }

        return $this->render('content/trainer_drill_form.html.twig', ['form' => $form, 'mode' => 'create']);
    }

    /**
     * AC-04-8: preview — full details, embedded video, "Add to Playlist".
     */
    #[Route('/trainer/content/drills/{drill<\d+>}', name: 'content_trainer_drill_show', methods: ['GET'])]
    public function show(Drill $drill): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_VIEW, $drill);

        return $this->render('content/trainer_drill_show.html.twig', ['drill' => $drill]);
    }

    #[Route('/trainer/content/drills/{drill<\d+>}/edit', name: 'content_trainer_drill_edit', methods: ['GET', 'POST'])]
    public function edit(Request $request, Drill $drill): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_EDIT, $drill);

        $form = $this->createForm(DrillType::class, [
            'title' => $drill->getTitle(),
            'youtubeUrl' => $drill->getYoutubeUrl(),
            'instructions' => $drill->getInstructions(),
            'tags' => null === $drill->getTags() ? null : implode(', ', $drill->getTags()),
            'difficultyLevel' => $drill->getDifficultyLevel(),
            'categories' => implode(', ', $drill->getCategories()),
            'equipment' => null === $drill->getEquipment() ? null : implode(', ', $drill->getEquipment()),
            'spaceRequirement' => $drill->getSpaceRequirement(),
            'playerCount' => $drill->getPlayerCount(),
            'durationMinMinutes' => $drill->getDurationMinMinutes(),
            'durationMaxMinutes' => $drill->getDurationMaxMinutes(),
            'isPublic' => $drill->isPublic(),
        ]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array<string, mixed> $data */
            $data = $form->getData();

            $this->drillService->updateDrill(
                $drill,
                (string) $data['title'],
                (string) $data['youtubeUrl'],
                $this->blankToNull($data['instructions'] ?? null),
                $this->parseCommaList($data['tags'] ?? null),
                (string) $data['difficultyLevel'],
                $this->parseCommaList($data['categories'] ?? null) ?? [],
                $this->parseCommaList($data['equipment'] ?? null),
                $this->blankToNull($data['spaceRequirement'] ?? null),
                $this->blankToNull($data['playerCount'] ?? null),
                '' === ($data['durationMinMinutes'] ?? '') ? null : (int) $data['durationMinMinutes'],
                '' === ($data['durationMaxMinutes'] ?? '') ? null : (int) $data['durationMaxMinutes'],
            );

            $this->addFlash('success', 'Drill updated.');

            return $this->redirectToRoute('content_trainer_drill_show', ['drill' => $drill->getId()]);
        }

        return $this->render('content/trainer_drill_form.html.twig', ['form' => $form, 'mode' => 'edit', 'drill' => $drill]);
    }

    /**
     * AC-04-35: "This drill is used in [N] playlists. Delete anyway?"
     */
    #[Route('/trainer/content/drills/{drill<\d+>}/delete', name: 'content_trainer_drill_delete', methods: ['GET', 'POST'])]
    public function delete(Request $request, Drill $drill): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_DELETE, $drill);
        $trainer = $this->currentTrainer();

        if ($request->isMethod('POST')) {
            if (!$this->isCsrfTokenValid('drill-delete'.$drill->getId(), $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            $this->drillService->delete($drill);
            $this->addFlash('success', sprintf('"%s" deleted.', $drill->getTitle()));

            return $this->redirectToRoute('content_trainer_drills_index');
        }

        return $this->render('content/trainer_drill_delete.html.twig', [
            'drill' => $drill,
            'usedInPlaylists' => $this->playlistItems->countPlaylistsUsingContentItem($trainer, $drill),
        ]);
    }

    /**
     * AC-04-17/18.
     */
    #[Route('/trainer/content/drills/{drill<\d+>}/visibility', name: 'content_trainer_drill_visibility', methods: ['GET', 'POST'])]
    public function visibility(Request $request, Drill $drill): Response
    {
        $this->denyAccessUnlessGranted(ContentItemVoter::CONTENT_ITEM_PUBLISH_TOGGLE, $drill);

        $form = $this->createForm(PublishToggleType::class, ['isPublic' => $drill->isPublic()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{isPublic: bool} $data */
            $data = $form->getData();

            $this->drillService->setPublic($drill, $data['isPublic']);
            $this->addFlash('success', $data['isPublic'] ? 'Drill is now public.' : 'Drill is now private.');

            return $this->redirectToRoute('content_trainer_drill_show', ['drill' => $drill->getId()]);
        }

        return $this->render('content/trainer_drill_visibility.html.twig', ['form' => $form, 'drill' => $drill]);
    }

    /**
     * AC-04-2: server-side YouTube metadata proxy — a structural stub, not a
     * real integration. No YouTube Data API key/config exists anywhere in
     * this project (no epic names one, and none of the settled specs
     * provision credentials for it), so this never fabricates a title or
     * duration; it returns nulls, honestly, matching the same "never claims
     * to do what it cannot" discipline as `NoopPaymentIntentGateway`. The
     * manual-entry path (`VideoItemType`/`DrillType`'s own fields) is what
     * actually satisfies AC-04-2's "duration... or entered manually".
     */
    #[Route('/trainer/content/youtube-metadata', name: 'content_trainer_youtube_metadata', methods: ['GET'])]
    public function youtubeMetadata(Request $request): Response
    {
        $url = (string) $request->query->get('url', '');
        $valid = 1 === preg_match(ContentItem::YOUTUBE_URL_PATTERN, $url);

        return $this->json(['title' => null, 'durationSeconds' => null, 'validFormat' => $valid]);
    }

    private function nullableString(mixed $value): ?string
    {
        return \is_string($value) && '' !== trim($value) ? $value : null;
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
}
