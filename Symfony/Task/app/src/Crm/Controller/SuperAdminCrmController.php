<?php

declare(strict_types=1);

namespace App\Crm\Controller;

use App\Crm\Entity\PlayerFlag;
use App\Crm\Exception\DuplicateActiveFlagException;
use App\Crm\Form\ApplyFlagType;
use App\Crm\Repository\PlayerFlagRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Crm\Repository\PlayerNoteRepository;
use App\Crm\Service\PlayerFlagService;
use App\Crm\Voter\PlayerVoter;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\CrossTenantReadService;
use App\Platform\Tenancy\AdministrativeScope;
use App\Scheduling\Repository\AttendanceRecordRepository;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-03.12 — Super Admin Views System-Wide CRM ("CRM Master"/"All Players").
 *
 * Lives in `Crm`, not `Administration` — `specs/api-designer-spec.md`
 * Decisions, "CRM Master / LPPP Analytics / referral-rule config placement":
 * "Each stays in its own epic's module, `/super-admin/...`-gated." The list
 * and dashboard read through `CrossTenantReadService` (no tenant needs to be
 * open for a read across all of them, mirroring Event Master's own list);
 * the detail view and flag writes open `AdministrativeScope` for the
 * membership's own trainer first, the same `withScope()` shape
 * `EventMasterController` already establishes, so `PlayerVoter` sees a real,
 * managed entity rather than a projection.
 *
 * **The write mechanism was genuinely unresolved upstream**
 * (`specs/security-voter-designer-design.md` Open questions #1: "cannot
 * state whether it gates on `AdministrativeScope` or on impersonation — no
 * route names the mechanism"). This controller resolves it as
 * `AdministrativeScope`, the same choice `PlayerVoter`'s own docblock
 * already made by analogy with `AttendanceVoter`'s structurally identical
 * gap — recorded here again since this is the controller that actually
 * exercises it.
 *
 * AC-03-57's revenue figure is deliberately absent from the dashboard here:
 * it needs Epic-05's `payment_record`, which does not exist in this
 * codebase. The non-revenue metrics (total players, sessions this week,
 * system-wide Top Players, system-wide flag counts) are built in full — see
 * the coder's final report.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-54..58
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class SuperAdminCrmController extends AbstractController
{
    public function __construct(
        private readonly CrossTenantReadService $crossTenantReads,
        private readonly TrainerRepository $trainers,
        private readonly AdministrativeScope $administrativeScope,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly PlayerLabelRepository $playerLabels,
        private readonly PlayerFlagRepository $playerFlags,
        private readonly PlayerNoteRepository $playerNotes,
        private readonly AttendanceRecordRepository $attendanceRecords,
        private readonly PlayerFlagService $playerFlagService,
    ) {
    }

    /**
     * AC-03-54: every player from every trainer, tool-specific search by
     * player/trainer/email. AC-03-55: filter by trainer, skill, age, gender,
     * flags, registration date range, last activity band (last-activity
     * banding is not derivable from this projection alone — see this
     * method's own filter list — so it is left to the detail view, matching
     * how CrossTenantReadService's other projections stay deliberately thin).
     */
    #[Route('/super-admin/crm/players', name: 'crm_super_admin_players_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $trainerId = $request->query->get('trainer');
        $registeredFrom = $request->query->get('registeredFrom');
        $registeredTo = $request->query->get('registeredTo');
        $flagTypes = $request->query->all('flags');

        $result = $this->crossTenantReads->listPlayers(
            actor: $this->actor(),
            query: $this->stringOrNull($request->query->get('q')),
            trainerId: null !== $trainerId && '' !== $trainerId ? (int) $trainerId : null,
            skillLevel: $this->stringOrNull($request->query->get('skillLevel')),
            minAge: $this->intOrNull($request->query->get('minAge')),
            maxAge: $this->intOrNull($request->query->get('maxAge')),
            gender: $this->stringOrNull($request->query->get('gender')),
            flagTypes: array_values(array_map('strval', $flagTypes)),
            registeredFrom: null !== $registeredFrom && '' !== $registeredFrom ? new \DateTimeImmutable((string) $registeredFrom) : null,
            registeredTo: null !== $registeredTo && '' !== $registeredTo ? new \DateTimeImmutable((string) $registeredTo) : null,
            page: max(1, $request->query->getInt('page', 1)),
        );

        return $this->render('crm/super_admin_players_index.html.twig', [
            'items' => $result['items'],
            'total' => $result['total'],
            'trainers' => $this->trainers->findAll(),
            'flagDefinitions' => PlayerFlag::labelsWithDefinitions(),
        ]);
    }

    /**
     * AC-03-56: apply flags across trainers, and (a player associated with
     * more than one trainer) view history across trainers — the trainer
     * list on this row IS that cross-trainer history, since
     * `PlayerTrainerMembership` is one row per (player, trainer) pair
     * (BR-03-2).
     */
    #[Route('/super-admin/crm/players/{membership<\d+>}', name: 'crm_super_admin_player_show', methods: ['GET'])]
    public function show(int $membership): Response
    {
        return $this->withScope($membership, function (PlayerTrainerMembership $membershipEntity): Response {
            $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_VIEW, $membershipEntity);

            $player = $membershipEntity->getPlayer();

            // AC-03-56: "view a player's history across trainers if the
            // player is associated with more than one" — `Trainer` itself
            // is a global, RLS-free table (specs/database-designer-schema.md
            // "Global tables"), so listing WHICH trainers needs no further
            // scope beyond the one already open for the membership being
            // viewed; each trainer's own detailed membership stays reachable
            // only by navigating to ITS OWN /super-admin/crm/players/{id}
            // (which opens that trainer's own scope in turn), never by
            // reading two tenants' rows inside one open scope.
            $otherTrainers = array_values(array_filter(
                array_map(
                    fn (int $trainerId) => $this->trainers->find($trainerId),
                    $this->crossTenantReads->trainerIdsForPlayer((int) $player->getId()),
                ),
                fn ($trainer) => null !== $trainer && $trainer->getId() !== $membershipEntity->getTrainer()->getId(),
            ));

            return $this->render('crm/super_admin_player_show.html.twig', [
                'membership' => $membershipEntity,
                'player' => $player,
                'otherTrainers' => $otherTrainers,
                'labels' => $this->playerLabels->findForPlayer($player),
                'activeFlags' => $this->playerFlags->findActiveForPlayer($player),
                'flagDefinitions' => PlayerFlag::labelsWithDefinitions(),
                'generalNotes' => $this->playerNotes->findGeneralForPlayer($player),
                'attendanceRecords' => \array_slice($this->attendanceRecords->findForPlayer($player), 0, 50),
                'flagForm' => $this->createForm(ApplyFlagType::class),
            ]);
        });
    }

    /**
     * AC-03-56/58: apply a flag across trainers.
     */
    #[Route('/super-admin/crm/players/{membership<\d+>}/flags', name: 'crm_super_admin_player_flag_add', methods: ['POST'])]
    public function flagAdd(Request $request, int $membership): Response
    {
        return $this->withScope($membership, function (PlayerTrainerMembership $membershipEntity) use ($request): Response {
            $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FLAG_MANAGE, $membershipEntity);

            $form = $this->createForm(ApplyFlagType::class);
            $form->handleRequest($request);

            if ($form->isSubmitted() && $form->isValid()) {
                /** @var array{flagType: string, note: ?string} $data */
                $data = $form->getData();

                try {
                    $this->playerFlagService->apply($membershipEntity, $data['flagType'], $this->actor(), '' === ($data['note'] ?? '') ? null : $data['note']);
                    $this->addFlash('success', 'Flag applied (Super Admin).');
                } catch (DuplicateActiveFlagException $e) {
                    $this->addFlash('error', $e->getMessage());
                }
            }

            return $this->redirectToRoute('crm_super_admin_player_show', ['membership' => $membershipEntity->getId()]);
        });
    }

    /**
     * AC-03-56/58: remove (resolve) a flag across trainers.
     */
    #[Route('/super-admin/crm/players/{membership<\d+>}/flags/{flag<\d+>}/resolve', name: 'crm_super_admin_player_flag_resolve', methods: ['POST'])]
    public function flagResolve(Request $request, int $membership, int $flag): Response
    {
        return $this->withScope($membership, function (PlayerTrainerMembership $membershipEntity) use ($request, $flag): Response {
            $this->denyAccessUnlessGranted(PlayerVoter::PLAYER_FLAG_MANAGE, $membershipEntity);

            if (!$this->isCsrfTokenValid('sa-player-flag-resolve'.$flag, $request->request->getString('_token'))) {
                throw $this->createAccessDeniedException('Invalid CSRF token.');
            }

            $flagEntity = $this->playerFlags->find($flag) ?? throw $this->createNotFoundException();
            $this->playerFlagService->resolve($flagEntity, $this->actor());
            $this->addFlash('success', 'Flag resolved (Super Admin).');

            return $this->redirectToRoute('crm_super_admin_player_show', ['membership' => $membershipEntity->getId()]);
        });
    }

    /**
     * AC-03-57: system-wide Quick View — total players, sessions this week,
     * system-wide Top Players, system-wide flag counts, drill-down by
     * trainer. No revenue figure — see this class's own docblock.
     */
    #[Route('/super-admin/crm/dashboard', name: 'crm_super_admin_dashboard', methods: ['GET'])]
    public function dashboard(Request $request): Response
    {
        $now = new \DateTimeImmutable();
        $weekStart = $now->modify(sprintf('-%d days', ((int) $now->format('N')) - 1))->setTime(0, 0);
        $weekEnd = $weekStart->modify('+7 days');

        $totals = $this->crossTenantReads->crmDashboardTotals($this->actor(), $weekStart, $weekEnd);
        $flagCounts = $this->crossTenantReads->flagCountsSystemWide();
        $filled = array_fill_keys(array_keys(PlayerFlag::labelsWithDefinitions()), 0);

        return $this->render('crm/super_admin_dashboard.html.twig', [
            'totalPlayers' => $totals['totalPlayers'],
            'sessionsThisWeek' => $totals['sessionsThisWeek'],
            'topPlayers' => $this->crossTenantReads->topPlayersSystemWide(10, $now),
            'flagCounts' => [...$filled, ...$flagCounts],
            'flagDefinitions' => PlayerFlag::labelsWithDefinitions(),
            'trainers' => $this->trainers->findAll(),
            'drillDownTrainerId' => $request->query->get('trainer'),
        ]);
    }

    /**
     * @template T
     *
     * @param callable(PlayerTrainerMembership): T $callback
     *
     * @return T
     */
    private function withScope(int $membershipId, callable $callback): mixed
    {
        $trainerId = $this->crossTenantReads->resolveTrainerIdForMembership($membershipId);

        if (null === $trainerId) {
            throw $this->createNotFoundException('Player not found.');
        }

        $trainer = $this->trainers->find($trainerId) ?? throw $this->createNotFoundException('Trainer not found.');

        $this->administrativeScope->openFor($trainer, $this->actor());

        try {
            $membershipEntity = $this->memberships->find($membershipId) ?? throw $this->createNotFoundException('Player not found.');

            return $callback($membershipEntity);
        } finally {
            $this->administrativeScope->close();
        }
    }

    private function stringOrNull(mixed $value): ?string
    {
        return \is_string($value) && '' !== $value ? $value : null;
    }

    private function intOrNull(mixed $value): ?int
    {
        return null === $value || '' === $value ? null : (int) $value;
    }

    private function actor(): Account
    {
        /** @var Account $account */
        $account = $this->getUser();

        return $account;
    }
}
