<?php

declare(strict_types=1);

namespace App\Crm\Controller;

use App\Crm\Dto\SegmentCriteria;
use App\Crm\Entity\Label;
use App\Crm\Entity\PlayerFlag;
use App\Crm\Entity\PlayerNote;
use App\Crm\Exception\DuplicateActiveFlagException;
use App\Crm\Exception\NoteEditWindowExpiredException;
use App\Crm\Form\AddNoteType;
use App\Crm\Form\ApplyFlagType;
use App\Crm\Form\ApplyLabelsType;
use App\Crm\Form\PlayerCrmFieldsType;
use App\Crm\Repository\LabelRepository;
use App\Crm\Repository\PlayerFlagRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Crm\Repository\PlayerNoteRepository;
use App\Crm\Repository\PlayerSegmentationRepository;
use App\Crm\Service\LabelService;
use App\Crm\Service\PlayerFlagService;
use App\Crm\Service\PlayerNoteService;
use App\Crm\Voter\PlayerVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Tenancy\TenantContext;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use App\Scheduling\Repository\AttendanceRecordRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-03.01, 02, 03 (player side), 04, 05, 06, 07: the trainer's player list,
 * search/segmentation, detail view, labels, flags, and notes.
 *
 * The Event History section's "+ Add Note" (AC-03-20) is rendered as ONE
 * page-level form with an event selector, not a separate form per table
 * row — the underlying, testable behavior (a note tied to a specific event,
 * displayed as "[Event Title] - [Date]: [Note]") is identical either way;
 * only the micro-interaction placement differs, a presentation choice left
 * for `coder-frontend`/`twig-ux-reviewer` to refine. Recorded in the
 * coder's final report.
 *
 * @see specs/api-designer-spec.md "Crm module" trainer console table
 */
#[IsGranted('ROLE_TRAINER')]
final class TrainerPlayerController extends AbstractController
{
    public function __construct(
        private readonly PlayerSegmentationRepository $segmentation,
        private readonly LabelRepository $labels,
        private readonly PlayerLabelRepository $playerLabels,
        private readonly PlayerFlagRepository $playerFlags,
        private readonly PlayerNoteRepository $playerNotes,
        private readonly AttendanceRecordRepository $attendanceRecords,
        private readonly LabelService $labelService,
        private readonly PlayerFlagService $playerFlagService,
        private readonly PlayerNoteService $playerNoteService,
        private readonly TenantContext $tenantContext,
        private readonly EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-03-1..10, AC-03-22..26, BR-03-13..15: the full roster, with
     * tool-specific search and AND-combined segmentation filters.
     */
    #[Route('/trainer/players', name: 'crm_trainer_players_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $criteria = $this->criteriaFromRequest($request);
        $result = $this->segmentation->search($this->tenantContext->requireTrainerId(), $criteria);

        return $this->render('crm/trainer_players_index.html.twig', [
            'items' => $result['items'],
            'total' => $result['total'],
            'criteria' => $criteria,
            'labels' => $this->labels->findAllForActiveTenant(),
            'flagDefinitions' => PlayerFlag::labelsWithDefinitions(),
        ]);
    }

    /**
     * AC-03-27..33: the full player detail.
     */
    #[Route('/trainer/players/{membership}', name: 'crm_trainer_player_show', methods: ['GET'])]
    public function show(PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_VIEW, $membership);

        return $this->render('crm/trainer_player_show.html.twig', $this->playerDetailContext($membership));
    }

    /**
     * AC-03-33: skill level and the other limited fields.
     */
    #[Route('/trainer/players/{membership}/edit', name: 'crm_trainer_player_edit', methods: ['POST'])]
    public function edit(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_EDIT, $membership);

        $form = $this->createForm(PlayerCrmFieldsType::class, ['skillLevel' => $membership->getSkillLevel()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{skillLevel: ?string} $data */
            $data = $form->getData();
            $membership->setSkillLevel('' === ($data['skillLevel'] ?? '') ? null : $data['skillLevel']);
            $this->entityManager->flush();
            $this->addFlash('success', 'Player profile updated.');

            return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
        }

        return $this->render('crm/trainer_player_show.html.twig', [...$this->playerDetailContext($membership), 'editForm' => $form]);
    }

    /**
     * AC-03-12: multi-select apply.
     */
    #[Route('/trainer/players/{membership}/labels', name: 'crm_trainer_player_label_add', methods: ['POST'])]
    public function labelAdd(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_LABEL_MANAGE, $membership);

        $form = $this->createForm(ApplyLabelsType::class, null, ['availableLabels' => $this->labels->findAllForActiveTenant()]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{labels: iterable<Label>} $data */
            $data = $form->getData();
            /** @var Account $actor */
            $actor = $this->getUser();

            foreach ($data['labels'] as $label) {
                $this->labelService->applyToPlayer($membership, $label, $actor);
            }

            $this->addFlash('success', 'Labels applied.');

            return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
        }

        return $this->render('crm/trainer_player_show.html.twig', [...$this->playerDetailContext($membership), 'applyLabelsForm' => $form]);
    }

    /**
     * AC-03-28: click-to-remove.
     */
    #[Route('/trainer/players/{membership}/labels/{label}/remove', name: 'crm_trainer_player_label_remove', methods: ['POST'])]
    public function labelRemove(Request $request, PlayerTrainerMembership $membership, Label $label): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_LABEL_MANAGE, $membership);
        $this->assertCsrf($request, 'player-label-remove'.$label->getId());

        $playerLabel = $this->playerLabels->findOneByPlayerAndLabel($membership->getPlayer(), $label);

        if (null !== $playerLabel) {
            $this->labelService->removeFromPlayer($playerLabel);
        }

        $this->addFlash('success', 'Label removed.');

        return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * AC-03-15: one of the 8 system flags, with an optional note.
     */
    #[Route('/trainer/players/{membership}/flags', name: 'crm_trainer_player_flag_add', methods: ['POST'])]
    public function flagAdd(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FLAG_MANAGE, $membership);

        $form = $this->createForm(ApplyFlagType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{flagType: string, note: ?string} $data */
            $data = $form->getData();
            /** @var Account $actor */
            $actor = $this->getUser();

            try {
                $this->playerFlagService->apply($membership, $data['flagType'], $actor, '' === ($data['note'] ?? '') ? null : $data['note']);
                $this->addFlash('success', 'Flag applied.');
            } catch (DuplicateActiveFlagException $e) {
                $this->addFlash('error', $e->getMessage());
            }

            return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
        }

        return $this->render('crm/trainer_player_show.html.twig', [...$this->playerDetailContext($membership), 'flagForm' => $form]);
    }

    /**
     * AC-03-17: "Mark as resolved?" — hides from the active view, keeps
     * history.
     */
    #[Route('/trainer/players/{membership}/flags/{flag}/resolve', name: 'crm_trainer_player_flag_resolve', methods: ['POST'])]
    public function flagResolve(Request $request, PlayerTrainerMembership $membership, PlayerFlag $flag): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FLAG_MANAGE, $membership);
        $this->assertCsrf($request, 'player-flag-resolve'.$flag->getId());

        /** @var Account $actor */
        $actor = $this->getUser();
        $this->playerFlagService->resolve($flag, $actor);
        $this->addFlash('success', 'Flag resolved.');

        return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * AC-03-19 (general) and AC-03-20 (per-event, when an event is selected).
     */
    #[Route('/trainer/players/{membership}/notes', name: 'crm_trainer_player_note_add', methods: ['POST'])]
    public function noteAdd(Request $request, PlayerTrainerMembership $membership): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $membership);

        $form = $this->createForm(AddNoteType::class, null, ['events' => $this->eventChoicesForPlayer($membership)]);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{text: string, event?: ?Event} $data */
            $data = $form->getData();
            /** @var Account $actor */
            $actor = $this->getUser();
            $event = $data['event'] ?? null;

            if (null !== $event) {
                $this->playerNoteService->addSessionNote($membership, $event, $actor, $data['text']);
            } else {
                $this->playerNoteService->addGeneral($membership, $actor, $data['text']);
            }

            $this->addFlash('success', 'Note added.');

            return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
        }

        return $this->render('crm/trainer_player_show.html.twig', [...$this->playerDetailContext($membership), 'noteForm' => $form]);
    }

    /**
     * AC-03-21/BR-03-12: within 24 hours of creation, and never on a
     * coach-authored note (PlayerVoter denies that outright).
     */
    #[Route('/trainer/players/{membership}/notes/{note}/edit', name: 'crm_trainer_player_note_edit', methods: ['POST'])]
    public function noteEdit(Request $request, PlayerTrainerMembership $membership, PlayerNote $note): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $note);

        $form = $this->createForm(AddNoteType::class);
        $form->handleRequest($request);

        if ($form->isSubmitted() && $form->isValid()) {
            /** @var array{text: string} $data */
            $data = $form->getData();
            /** @var Account $actor */
            $actor = $this->getUser();

            try {
                $this->playerNoteService->edit($note, $actor, $data['text'], new \DateTimeImmutable());
                $this->addFlash('success', 'Note updated.');
            } catch (NoteEditWindowExpiredException $e) {
                $this->addFlash('error', $e->getMessage());
            }
        }

        return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * AC-03-21: at any time, for the trainer's own notes.
     */
    #[Route('/trainer/players/{membership}/notes/{note}/delete', name: 'crm_trainer_player_note_delete', methods: ['POST'])]
    public function noteDelete(Request $request, PlayerTrainerMembership $membership, PlayerNote $note): Response
    {
        $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_NOTE_MANAGE, $note);
        $this->assertCsrf($request, 'player-note-delete'.$note->getId());

        $this->playerNoteService->delete($note);
        $this->addFlash('success', 'Note deleted.');

        return $this->redirectToRoute('crm_trainer_player_show', ['membership' => $membership->getId()]);
    }

    /**
     * @return array<string, mixed>
     */
    private function playerDetailContext(PlayerTrainerMembership $membership): array
    {
        $player = $membership->getPlayer();
        $attendance = $this->attendanceRecords->findForPlayer($player);

        $attendedCount = 0;
        $noShowCount = 0;
        $lastEventAt = null;

        foreach ($attendance as $record) {
            if (\in_array($record->getStatus(), [AttendanceRecord::STATUS_PRESENT, AttendanceRecord::STATUS_LATE], true)) {
                ++$attendedCount;
            }

            if (AttendanceRecord::STATUS_ABSENT === $record->getStatus()) {
                ++$noShowCount;
            }

            if (null === $lastEventAt || $record->getEvent()->getStartsAt() > $lastEventAt) {
                $lastEventAt = $record->getEvent()->getStartsAt();
            }
        }

        $sessionNotes = $this->playerNotes->findSessionNotesForPlayer($player);
        $coachFeedback = array_values(array_filter(
            $sessionNotes,
            static fn (PlayerNote $note): bool => AccountRole::Coach === $note->getCreatedByAccount()->getRole(),
        ));
        $trainerEventNotes = array_values(array_filter(
            $sessionNotes,
            static fn (PlayerNote $note): bool => AccountRole::Coach !== $note->getCreatedByAccount()->getRole(),
        ));

        return [
            'now' => new \DateTimeImmutable(),
            'membership' => $membership,
            'player' => $player,
            'labels' => $this->playerLabels->findForPlayer($player),
            'activeFlags' => $this->playerFlags->findActiveForPlayer($player),
            'flagDefinitions' => PlayerFlag::labelsWithDefinitions(),
            'generalNotes' => $this->playerNotes->findGeneralForPlayer($player),
            'eventNotes' => $trainerEventNotes,
            'coachFeedback' => $coachFeedback,
            'attendanceRecords' => \array_slice($attendance, 0, 50),
            'attendedCount' => $attendedCount,
            'totalTrackedCount' => \count($attendance),
            'noShowCount' => $noShowCount,
            'lastEventAt' => $lastEventAt,
            'editForm' => $this->createForm(PlayerCrmFieldsType::class, ['skillLevel' => $membership->getSkillLevel()]),
            'applyLabelsForm' => $this->createForm(ApplyLabelsType::class, null, ['availableLabels' => $this->labels->findAllForActiveTenant()]),
            'flagForm' => $this->createForm(ApplyFlagType::class),
            'noteForm' => $this->createForm(AddNoteType::class, null, ['events' => $this->eventChoicesForPlayer($membership)]),
        ];
    }

    /**
     * @return array<int, Event>
     */
    private function eventChoicesForPlayer(PlayerTrainerMembership $membership): array
    {
        $choices = [];

        foreach ($this->attendanceRecords->findForPlayer($membership->getPlayer()) as $record) {
            $event = $record->getEvent();
            $choices[(int) $event->getId()] = $event;
        }

        return $choices;
    }

    private function criteriaFromRequest(Request $request): SegmentCriteria
    {
        $q = $request->query->get('q');
        $labelIds = $request->query->all('labels');
        $flagTypes = $request->query->all('flags');
        $registeredFrom = $request->query->get('registeredFrom');
        $registeredTo = $request->query->get('registeredTo');

        return new SegmentCriteria(
            query: $this->nullableString($q),
            skillLevel: $this->nullableString($request->query->get('skillLevel')),
            minAge: $this->nullableInt($request->query->get('minAge')),
            maxAge: $this->nullableInt($request->query->get('maxAge')),
            gender: $this->nullableString($request->query->get('gender')),
            labelIds: [] === $labelIds ? null : array_values(array_map('intval', $labelIds)),
            flagTypes: [] === $flagTypes ? null : array_values(array_map('strval', $flagTypes)),
            teamSchoolClub: $this->nullableString($request->query->get('teamSchoolClub')),
            attendedMoreThan: $this->nullableInt($request->query->get('attendedMoreThan')),
            attendedInLastDays: $this->nullableInt($request->query->get('attendedInLastDays')),
            attendanceRateGreaterThan: null !== $request->query->get('attendanceRateGreaterThan') ? (float) $request->query->get('attendanceRateGreaterThan') : null,
            noShowsGreaterThan: $this->nullableInt($request->query->get('noShowsGreaterThan')),
            registeredFrom: null !== $registeredFrom && '' !== $registeredFrom ? new \DateTimeImmutable((string) $registeredFrom) : null,
            registeredTo: null !== $registeredTo && '' !== $registeredTo ? new \DateTimeImmutable((string) $registeredTo) : null,
            lastActivityBand: $this->nullableString($request->query->get('lastActivityBand')),
            sort: (string) $request->query->get('sort', SegmentCriteria::SORT_NAME_ASC),
            page: max(1, $request->query->getInt('page', 1)),
        );
    }

    private function nullableString(mixed $value): ?string
    {
        return null === $value || '' === $value ? null : (string) $value;
    }

    private function nullableInt(mixed $value): ?int
    {
        return null === $value || '' === $value ? null : (int) $value;
    }

    private function assertCsrf(Request $request, string $id): void
    {
        if (!$this->isCsrfTokenValid($id, $request->request->getString('_token'))) {
            throw $this->createAccessDeniedException('Invalid CSRF token.');
        }
    }
}
