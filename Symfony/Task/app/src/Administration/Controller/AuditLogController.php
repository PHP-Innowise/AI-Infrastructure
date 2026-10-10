<?php

declare(strict_types=1);

namespace App\Administration\Controller;

use App\Platform\Repository\AuditLogEntryRepository;
use App\Platform\Voter\AuditVoter;
use Symfony\Bundle\FrameworkBundle\Controller\AbstractController;
use Symfony\Component\HttpFoundation\Request;
use Symfony\Component\HttpFoundation\Response;
use Symfony\Component\HttpFoundation\StreamedResponse;
use Symfony\Component\Routing\Attribute\Route;
use Symfony\Component\Security\Http\Attribute\IsGranted;

/**
 * US-07.08: the Audit Log — AC-07-29..34, BR-07-4..6.
 *
 * BR-07-5 (negative rule): viewing this screen is itself explicitly
 * excluded from the log as "too verbose" — neither action here calls
 * `AuditLogger`, matching `specs/api-designer-spec.md:706-714`'s own note
 * that `administration_audit_log_index` "write[s] no audit entry on their
 * own GET."
 *
 * AC-07-34/BR-07-6 (retention): satisfied structurally, not by a purge
 * job — `audit_log_entry`'s append-only database privilege (`REVOKE
 * UPDATE, DELETE ... FROM pp_app`, Version20260810090000) means the
 * application role can never delete an entry at all, so every entry is
 * retained indefinitely by construction, which trivially satisfies "at
 * least 1 year." No Scheduler task purges this table.
 *
 * @see specs/api-designer-spec.md "Administration module"
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md AC-07-29..34
 */
#[IsGranted('ROLE_SUPER_ADMIN')]
final class AuditLogController extends AbstractController
{
    private const PER_PAGE = 50;

    public function __construct(
        private readonly AuditLogEntryRepository $auditLog,
    ) {
    }

    /**
     * AC-07-29: chronological list. AC-07-32: filtered by date range
     * (last 7/30 days, custom), action type, and searched by subject (user
     * or trainer name).
     */
    #[Route('/super-admin/audit-log', name: 'administration_audit_log_index', methods: ['GET'])]
    public function index(Request $request): Response
    {
        $this->denyAccessUnlessGranted(AuditVoter::AUDIT_LOG_VIEW);

        [$actionType, $dateFrom, $dateTo, $q] = $this->filtersFromRequest($request);
        $page = max(1, (int) $request->query->get('page', 1));

        $entries = $this->auditLog->search(
            actionType: $actionType,
            limit: self::PER_PAGE,
            offset: (self::PER_PAGE) * ($page - 1),
            occurredFrom: $dateFrom,
            occurredTo: $dateTo,
            subjectQuery: $q,
        );
        $total = $this->auditLog->countMatching(
            actionType: $actionType,
            occurredFrom: $dateFrom,
            occurredTo: $dateTo,
            subjectQuery: $q,
        );

        return $this->render('administration/audit_log_index.html.twig', [
            'entries' => $entries,
            'total' => $total,
            'page' => $page,
            'perPage' => self::PER_PAGE,
            'actionType' => $actionType,
            'dateFrom' => $dateFrom,
            'dateTo' => $dateTo,
            'q' => $q,
        ]);
    }

    /**
     * AC-07-33: CSV export, same filters, streamed.
     */
    #[Route('/super-admin/audit-log/export', name: 'administration_audit_log_export', methods: ['GET'])]
    public function export(Request $request): StreamedResponse
    {
        $this->denyAccessUnlessGranted(AuditVoter::AUDIT_LOG_VIEW);

        [$actionType, $dateFrom, $dateTo, $q] = $this->filtersFromRequest($request);

        // A generous cap, not "everything ever" — mirrors
        // EventMasterController::export()'s own 100000 ceiling exactly.
        $entries = $this->auditLog->search(
            actionType: $actionType,
            limit: 100000,
            occurredFrom: $dateFrom,
            occurredTo: $dateTo,
            subjectQuery: $q,
        );

        $response = new StreamedResponse(function () use ($entries): void {
            $out = fopen('php://output', 'w');
            \assert(false !== $out);
            fputcsv($out, ['Timestamp', 'Action', 'Subject Type', 'Subject Id', 'Trainer', 'Actor', 'Details'], escape: '\\');

            foreach ($entries as $entry) {
                fputcsv($out, [
                    $entry->getOccurredAt()->format('Y-m-d H:i:s'),
                    $entry->getActionType(),
                    $entry->getSubjectType(),
                    $entry->getSubjectId(),
                    $entry->getRelatedTrainer()?->getBusinessName(),
                    $entry->getActorAccount()?->getEmail() ?? 'system',
                    json_encode($entry->getDetails(), \JSON_THROW_ON_ERROR),
                ], escape: '\\');
            }

            fclose($out);
        });

        $response->headers->set('Content-Type', 'text/csv');
        $response->headers->set('Content-Disposition', 'attachment; filename="audit-log.csv"');

        return $response;
    }

    /**
     * @return array{0: ?string, 1: ?\DateTimeImmutable, 2: ?\DateTimeImmutable, 3: ?string}
     */
    private function filtersFromRequest(Request $request): array
    {
        $actionType = $request->query->get('actionType');
        $dateFrom = $request->query->get('dateFrom');
        $dateTo = $request->query->get('dateTo');
        $q = $request->query->get('q');

        return [
            \is_string($actionType) && '' !== $actionType ? $actionType : null,
            \is_string($dateFrom) && '' !== $dateFrom ? new \DateTimeImmutable($dateFrom) : null,
            // Inclusive of the whole end day: a bare date at midnight would
            // otherwise exclude every entry from that day, matching
            // EventMasterController's own dateTo handling shape (though
            // that one compares starts_at <= dateTo; this table's own
            // occurredAt needs the "< next day" form to be truly inclusive
            // of a same-day entry recorded at any hour).
            \is_string($dateTo) && '' !== $dateTo ? (new \DateTimeImmutable($dateTo))->modify('+1 day') : null,
            \is_string($q) && '' !== $q ? $q : null,
        ];
    }
}
