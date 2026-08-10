<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Identity\Entity\Account;
use Doctrine\DBAL\Connection;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\DependencyInjection\Attribute\Autowire;

/**
 * "The only cross-tenant read." Uses the separate, read-only `crossing`
 * DBAL connection — `BYPASSRLS`, `SELECT`-only — which has no ORM entity
 * manager behind it, so it is structurally incapable of returning a managed
 * entity: every method here returns plain arrays. Every call writes an
 * audit entry (architect-architecture.md "Crossing the boundary": "Every
 * crossing read writes an audit entry").
 *
 * First built here, in Epic-02, for the Super Admin Event Master minimal
 * slice (AC-02-55..57) — see AdministrativeScope's own docblock for why
 * this lands in `Platform` rather than `Administration`, which does not
 * exist as a full console yet.
 *
 * @see specs/architect-architecture.md "Crossing the boundary"
 * @see specs/database-designer-schema.md "Row-Level Security"
 */
final readonly class CrossTenantReadService
{
    public function __construct(
        #[Autowire(service: 'doctrine.dbal.crossing_connection')]
        private Connection $crossing,
        private AuditLogger $auditLogger,
        private EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * AC-02-55/56: every trainer's events, with tool-specific search
     * (title, trainer name, location, date) and filters (trainer, location,
     * date range, type, status), 50 per page.
     *
     * Status derivation matches Event::displayStatus() exactly: "completed"
     * is never stored, only derived at read time (active + already started).
     *
     * @return array{items: list<array<string, mixed>>, total: int}
     */
    public function listEvents(
        Account $actor,
        ?string $query,
        ?int $trainerId,
        ?string $location,
        ?\DateTimeImmutable $dateFrom,
        ?\DateTimeImmutable $dateTo,
        ?string $eventType,
        ?string $status,
        int $page,
        int $perPage = 50,
    ): array {
        $conditions = [];
        $params = [];

        if (null !== $query && '' !== trim($query)) {
            $conditions[] = '(e.title ILIKE :query OR t.business_name ILIKE :query OR e.location ILIKE :query)';
            $params['query'] = '%'.$query.'%';
        }

        if (null !== $trainerId) {
            $conditions[] = 'e.trainer_id = :trainerId';
            $params['trainerId'] = $trainerId;
        }

        if (null !== $location && '' !== trim($location)) {
            $conditions[] = 'e.location ILIKE :location';
            $params['location'] = '%'.$location.'%';
        }

        if (null !== $dateFrom) {
            $conditions[] = 'e.starts_at >= :dateFrom';
            $params['dateFrom'] = $dateFrom->format('Y-m-d H:i:sP');
        }

        if (null !== $dateTo) {
            $conditions[] = 'e.starts_at <= :dateTo';
            $params['dateTo'] = $dateTo->format('Y-m-d H:i:sP');
        }

        if (null !== $eventType && '' !== $eventType) {
            $conditions[] = 'e.event_type = :eventType';
            $params['eventType'] = $eventType;
        }

        if (null !== $status && '' !== $status) {
            $conditions[] = "(CASE WHEN e.status = 'active' AND e.starts_at <= NOW() THEN 'completed' ELSE e.status END) = :status";
            $params['status'] = $status;
        }

        $where = [] === $conditions ? '' : 'WHERE '.implode(' AND ', $conditions);
        $offset = max(0, $page - 1) * $perPage;

        $items = $this->crossing->fetchAllAssociative(
            <<<SQL
                SELECT
                    e.id, e.title, e.event_type, e.starts_at, e.ends_at, e.location,
                    e.capacity, e.visibility,
                    (CASE WHEN e.status = 'active' AND e.starts_at <= NOW() THEN 'completed' ELSE e.status END) AS display_status,
                    e.trainer_id, t.business_name AS trainer_name
                FROM event e
                JOIN trainer t ON t.id = e.trainer_id
                {$where}
                ORDER BY e.starts_at DESC
                LIMIT :limit OFFSET :offset
                SQL,
            [...$params, 'limit' => $perPage, 'offset' => $offset],
        );

        $total = (int) $this->crossing->fetchOne(
            "SELECT COUNT(*) FROM event e JOIN trainer t ON t.id = e.trainer_id {$where}",
            $params,
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.event_master_index', 'Event', null, null, [
            'query' => $query,
            'trainerId' => $trainerId,
            'page' => $page,
        ]);
        $this->entityManager->flush();

        return ['items' => $items, 'total' => $total];
    }

    /**
     * AC-02-57: which trainer a given event id belongs to — resolved via
     * the crossing connection since, at the point Event Master needs this
     * (before AdministrativeScope has opened any tenant), no tenant is
     * active and the ordinary RLS-bound connection would see nothing at
     * all. This is the one place a crossing read returns a bare scalar
     * rather than a projection — it exists only to open the *real* scope,
     * never to stand in for one.
     */
    public function resolveTrainerIdForEvent(int $eventId): ?int
    {
        $trainerId = $this->crossing->fetchOne('SELECT trainer_id FROM event WHERE id = :id', ['id' => $eventId]);

        return false === $trainerId ? null : (int) $trainerId;
    }
}
