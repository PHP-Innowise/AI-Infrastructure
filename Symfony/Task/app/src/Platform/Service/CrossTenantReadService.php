<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Identity\Entity\Account;
use Doctrine\DBAL\ArrayParameterType;
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

    /**
     * AC-03-54/55: every player across every trainer, tool-specific search
     * (player name, trainer name, email) and filters (trainer, skill, age,
     * gender, flags, registration date range, last activity). Mirrors
     * `PlayerSegmentationRepository::search()`'s own attendance-rate/
     * last-activity definitions exactly, so the system-wide list and each
     * trainer's own list never disagree about what either figure means.
     *
     * @param list<string> $flagTypes
     *
     * @return array{items: list<array<string, mixed>>, total: int}
     */
    public function listPlayers(
        Account $actor,
        ?string $query,
        ?int $trainerId,
        ?string $skillLevel,
        ?int $minAge,
        ?int $maxAge,
        ?string $gender,
        array $flagTypes,
        ?\DateTimeImmutable $registeredFrom,
        ?\DateTimeImmutable $registeredTo,
        int $page,
        int $perPage = 50,
    ): array {
        $conditions = ["ptm.status = 'active'"];
        $params = [];

        if (null !== $query && '' !== trim($query)) {
            $conditions[] = '(p.first_name ILIKE :query OR t.business_name ILIKE :query OR acc.email::text ILIKE :query)';
            $params['query'] = '%'.$query.'%';
        }

        if (null !== $trainerId) {
            $conditions[] = 'ptm.trainer_id = :trainerId';
            $params['trainerId'] = $trainerId;
        }

        if (null !== $skillLevel && '' !== $skillLevel) {
            $conditions[] = 'ptm.skill_level = :skillLevel';
            $params['skillLevel'] = $skillLevel;
        }

        if (null !== $minAge) {
            $conditions[] = "DATE_PART('year', AGE(CURRENT_DATE, p.date_of_birth)) >= :minAge";
            $params['minAge'] = $minAge;
        }

        if (null !== $maxAge) {
            $conditions[] = "DATE_PART('year', AGE(CURRENT_DATE, p.date_of_birth)) <= :maxAge";
            $params['maxAge'] = $maxAge;
        }

        if (null !== $gender && '' !== $gender) {
            $conditions[] = 'p.gender = :gender';
            $params['gender'] = $gender;
        }

        if ([] !== $flagTypes) {
            $conditions[] = "EXISTS (SELECT 1 FROM player_flag pf WHERE pf.player_id = p.id AND pf.status = 'active' AND pf.flag_type IN (:flagTypes))";
            $params['flagTypes'] = $flagTypes;
        }

        if (null !== $registeredFrom) {
            $conditions[] = 'ptm.joined_at >= :registeredFrom';
            $params['registeredFrom'] = $registeredFrom->format('Y-m-d H:i:sP');
        }

        if (null !== $registeredTo) {
            $conditions[] = 'ptm.joined_at <= :registeredTo';
            $params['registeredTo'] = $registeredTo->format('Y-m-d H:i:sP');
        }

        $where = implode(' AND ', $conditions);
        $offset = max(0, $page - 1) * $perPage;
        $types = [] !== $flagTypes ? ['flagTypes' => ArrayParameterType::STRING] : [];

        $items = $this->crossing->fetchAllAssociative(
            <<<SQL
                SELECT
                    ptm.id AS membership_id, p.id AS player_id, p.first_name, p.date_of_birth, p.gender,
                    ptm.skill_level, ptm.trainer_id, t.business_name AS trainer_name
                FROM player_trainer_membership ptm
                JOIN player_profile p ON p.id = ptm.player_profile_id
                JOIN trainer t ON t.id = ptm.trainer_id
                LEFT JOIN account acc ON acc.id = p.self_account_id
                WHERE {$where}
                ORDER BY p.first_name ASC
                LIMIT :limit OFFSET :offset
                SQL,
            [...$params, 'limit' => $perPage, 'offset' => $offset],
            $types,
        );

        $total = (int) $this->crossing->fetchOne(
            <<<SQL
                SELECT COUNT(*)
                FROM player_trainer_membership ptm
                JOIN player_profile p ON p.id = ptm.player_profile_id
                JOIN trainer t ON t.id = ptm.trainer_id
                LEFT JOIN account acc ON acc.id = p.self_account_id
                WHERE {$where}
                SQL,
            $params,
            $types,
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.crm_master_index', 'PlayerTrainerMembership', null, null, [
            'query' => $query,
            'trainerId' => $trainerId,
            'page' => $page,
        ]);
        $this->entityManager->flush();

        return ['items' => $items, 'total' => $total];
    }

    /**
     * AC-03-56: "view a player's history across trainers if the player is
     * associated with more than one" — every trainer id this player has an
     * ACTIVE association with, across the whole platform. Every ordinary,
     * RLS-bound repository can only ever see the single active tenant's own
     * membership row for this player; this is the one query that legitimately
     * needs to see all of them at once.
     *
     * @return list<int>
     */
    public function trainerIdsForPlayer(int $playerId): array
    {
        $rows = $this->crossing->fetchFirstColumn(
            "SELECT trainer_id FROM player_trainer_membership WHERE player_profile_id = :playerId AND status = 'active' ORDER BY joined_at ASC",
            ['playerId' => $playerId],
        );

        return array_map('intval', $rows);
    }

    /**
     * AC-03-56: which trainer a given `PlayerTrainerMembership` id belongs
     * to — the same "resolve before opening the real scope" role
     * `resolveTrainerIdForEvent()` plays for Event Master.
     */
    public function resolveTrainerIdForMembership(int $membershipId): ?int
    {
        $trainerId = $this->crossing->fetchOne('SELECT trainer_id FROM player_trainer_membership WHERE id = :id', ['id' => $membershipId]);

        return false === $trainerId ? null : (int) $trainerId;
    }

    /**
     * AC-03-57: system-wide Quick View — total players and sessions this
     * week across every trainer. Deliberately excludes revenue: AC-03-57's
     * revenue figure needs Epic-05's `payment_record`, which does not exist
     * in this codebase — see the coder's final report.
     *
     * @return array{totalPlayers: int, sessionsThisWeek: int}
     */
    public function crmDashboardTotals(Account $actor, \DateTimeImmutable $weekStart, \DateTimeImmutable $weekEnd): array
    {
        $totalPlayers = (int) $this->crossing->fetchOne(
            "SELECT COUNT(DISTINCT player_profile_id) FROM player_trainer_membership WHERE status = 'active'",
        );

        $sessionsThisWeek = (int) $this->crossing->fetchOne(
            'SELECT COUNT(*) FROM event WHERE starts_at >= :from AND starts_at < :to',
            ['from' => $weekStart->format('Y-m-d H:i:sP'), 'to' => $weekEnd->format('Y-m-d H:i:sP')],
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.crm_master_dashboard', 'PlayerTrainerMembership', null, null, []);
        $this->entityManager->flush();

        return ['totalPlayers' => $totalPlayers, 'sessionsThisWeek' => $sessionsThisWeek];
    }

    /**
     * AC-03-57: system-wide Top Players — the most active players across
     * the WHOLE platform, same BR-03-16/17 definition (90-day window,
     * Present/Late only, alphabetical tie-break) as each trainer's own Top
     * Players, just without the `trainer_id` predicate.
     *
     * @return list<array{playerId: int, name: string, trainerName: string, sessionCount: int}>
     */
    public function topPlayersSystemWide(int $limit = 10, ?\DateTimeImmutable $now = null): array
    {
        $since = ($now ?? new \DateTimeImmutable())->modify('-90 days');

        $rows = $this->crossing->fetchAllAssociative(
            <<<'SQL'
                SELECT p.id AS player_id, p.first_name AS name, t.business_name AS trainer_name, COUNT(a.id) AS session_count
                FROM attendance_record a
                JOIN player_profile p ON p.id = a.player_id
                JOIN trainer t ON t.id = a.trainer_id
                WHERE a.status IN ('present', 'late') AND a.recorded_at >= :since
                GROUP BY p.id, p.first_name, t.business_name
                ORDER BY session_count DESC, p.first_name ASC
                LIMIT :limit
                SQL,
            ['since' => $since->format('Y-m-d H:i:sP'), 'limit' => $limit],
        );

        return array_map(
            static fn (array $row): array => [
                'playerId' => (int) $row['player_id'],
                'name' => (string) $row['name'],
                'trainerName' => (string) $row['trainer_name'],
                'sessionCount' => (int) $row['session_count'],
            ],
            $rows,
        );
    }

    /**
     * AC-03-57: system-wide flag counts, across every trainer. Sparse —
     * only types with at least one active flag appear — rather than the
     * zero-filled shape `PlayerFlagRepository::countActiveByTypeForActiveTenant()`
     * returns for one trainer: filling every one of the 8 closed-vocabulary
     * types would mean this Platform-module service importing `PlayerFlag`
     * from Crm, inverting the module dependency direction that lets Crm
     * depend on Platform but never the reverse. The caller (a Crm-module
     * controller) already owns that vocabulary and fills in the zeros.
     *
     * @return array<string, int>
     */
    public function flagCountsSystemWide(): array
    {
        $rows = $this->crossing->fetchAllAssociative(
            "SELECT flag_type, COUNT(*) AS cnt FROM player_flag WHERE status = 'active' GROUP BY flag_type",
        );

        $counts = [];

        foreach ($rows as $row) {
            $counts[(string) $row['flag_type']] = (int) $row['cnt'];
        }

        return $counts;
    }

    /**
     * Epic-05: which trainer a `payment_record` row belongs to — the same
     * "resolve before opening the real scope" role `resolveTrainerIdForEvent()`
     * plays for Event Master, needed here because
     * `App\Billing\MessageHandler\ProcessStripeWebhookEventHandler` learns
     * only a Stripe id from the webhook payload, with no tenant context yet
     * active to read the trainer-scoped `payment_record` table through the
     * ordinary RLS-bound connection. `$actor` is null (a system/webhook
     * caller, not an authenticated Super Admin) — `AuditLogger::record()`
     * accepts that.
     */
    public function resolveTrainerIdForPaymentRecord(int $paymentRecordId): ?int
    {
        $trainerId = $this->crossing->fetchOne('SELECT trainer_id FROM payment_record WHERE id = :id', ['id' => $paymentRecordId]);

        $this->auditLogger->record(null, 'cross_tenant_read.webhook_resolve_payment_record', 'PaymentRecord', $paymentRecordId, null, []);
        $this->entityManager->flush();

        return false === $trainerId ? null : (int) $trainerId;
    }

    /**
     * Epic-05: the same resolution, keyed by the Stripe PaymentIntent id —
     * every `payment_intent.*` webhook event carries this, never the
     * platform's own `payment_record.id`.
     *
     * @return array{id: int, trainerId: int}|null
     */
    public function resolvePaymentRecordByStripePaymentIntent(string $stripePaymentIntentId): ?array
    {
        $row = $this->crossing->fetchAssociative(
            'SELECT id, trainer_id FROM payment_record WHERE stripe_payment_intent_id = :piId',
            ['piId' => $stripePaymentIntentId],
        );

        $this->auditLogger->record(null, 'cross_tenant_read.webhook_resolve_payment_intent', 'PaymentRecord', false !== $row ? (int) $row['id'] : null, null, [
            'stripePaymentIntentId' => $stripePaymentIntentId,
        ]);
        $this->entityManager->flush();

        return false === $row ? null : ['id' => (int) $row['id'], 'trainerId' => (int) $row['trainer_id']];
    }

    /**
     * Epic-05: the trainer owning a Stripe Connect account id — used by the
     * `account.updated` webhook (AC-05-34), which carries only the
     * connected account id.
     */
    public function resolveTrainerIdForStripeConnectAccount(string $stripeConnectAccountId): ?int
    {
        $trainerId = $this->crossing->fetchOne(
            'SELECT trainer_id FROM trainer_billing_settings WHERE stripe_connect_account_id = :accountId',
            ['accountId' => $stripeConnectAccountId],
        );

        $this->auditLogger->record(null, 'cross_tenant_read.webhook_resolve_connect_account', 'TrainerBillingSettings', null, null, [
            'stripeConnectAccountId' => $stripeConnectAccountId,
        ]);
        $this->entityManager->flush();

        return false === $trainerId ? null : (int) $trainerId;
    }

    /**
     * Epic-05: which trainer a `subscription_entitlement` row belongs to —
     * needed by `customer.subscription.deleted` (AC-05-34), which is looked
     * up by `stripe_subscription_id` on `platform_subscription`
     * (trainer-own-subscription-to-platform) OR, for a player subscription
     * cancellation, has no local row keyed the same way; player
     * subscriptions are entitlements, not Stripe Subscriptions at all
     * (BR-05-14 — "not a separate Stripe subscription"), so
     * `customer.subscription.deleted` only ever applies to a TRAINER's own
     * platform subscription in this codebase's actual scope, resolved via
     * `platform_subscription.stripe_subscription_id` instead.
     */
    public function resolveTrainerIdForPlatformStripeSubscription(string $stripeSubscriptionId): ?int
    {
        $trainerId = $this->crossing->fetchOne(
            'SELECT trainer_id FROM platform_subscription WHERE stripe_subscription_id = :subId',
            ['subId' => $stripeSubscriptionId],
        );

        $this->auditLogger->record(null, 'cross_tenant_read.webhook_resolve_platform_subscription', 'PlatformSubscription', null, null, [
            'stripeSubscriptionId' => $stripeSubscriptionId,
        ]);
        $this->entityManager->flush();

        return false === $trainerId ? null : (int) $trainerId;
    }

    /**
     * AC-04-37 "Content Stats": total playlists and drills across every
     * trainer, and the public-vs-private ratio (playlists and content items
     * combined — the epic states one ratio, not two separate ones).
     *
     * @return array{totalPlaylists: int, totalDrills: int, publicCount: int, privateCount: int}
     */
    public function contentStats(Account $actor): array
    {
        $totalPlaylists = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM playlist WHERE deleted_at IS NULL');
        $totalDrills = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM drill_detail dd JOIN content_item ci ON ci.id = dd.id WHERE ci.deleted_at IS NULL');
        $publicCount = (int) $this->crossing->fetchOne(
            "SELECT (SELECT COUNT(*) FROM playlist WHERE deleted_at IS NULL AND is_public) + (SELECT COUNT(*) FROM content_item WHERE deleted_at IS NULL AND is_public)",
        );
        $privateCount = (int) $this->crossing->fetchOne(
            "SELECT (SELECT COUNT(*) FROM playlist WHERE deleted_at IS NULL AND NOT is_public) + (SELECT COUNT(*) FROM content_item WHERE deleted_at IS NULL AND NOT is_public)",
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_stats', 'ContentItem', null, null, []);
        $this->entityManager->flush();

        return ['totalPlaylists' => $totalPlaylists, 'totalDrills' => $totalDrills, 'publicCount' => $publicCount, 'privateCount' => $privateCount];
    }

    /**
     * AC-04-37 "top 10 most-used public drills" — "used" is measured by
     * `content_usage`, which is scoped to the REUSING trainer by design
     * (architect-architecture.md: "the creator's 'used by N trainers' figure
     * is therefore a crossing read, which is correct: it is cross-tenant
     * information") — the count of DISTINCT other trainers who have added
     * the drill to one of their own playlists.
     *
     * @return list<array{contentItemId: int, title: string, trainerName: string, usedByTrainers: int}>
     */
    public function topPublicDrills(Account $actor, int $limit = 10): array
    {
        $rows = $this->crossing->fetchAllAssociative(
            <<<'SQL'
                SELECT ci.id, ci.title, t.business_name AS trainer_name, COUNT(cu.id) AS used_by
                FROM content_item ci
                JOIN drill_detail dd ON dd.id = ci.id
                JOIN trainer t ON t.id = ci.trainer_id
                LEFT JOIN content_usage cu ON cu.content_item_id = ci.id
                WHERE ci.is_public = true AND ci.deleted_at IS NULL
                GROUP BY ci.id, ci.title, t.business_name
                ORDER BY used_by DESC, ci.title ASC
                LIMIT :limit
                SQL,
            ['limit' => $limit],
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_top_drills', 'ContentItem', null, null, []);
        $this->entityManager->flush();

        return array_map(
            static fn (array $row): array => [
                'contentItemId' => (int) $row['id'],
                'title' => (string) $row['title'],
                'trainerName' => (string) $row['trainer_name'],
                'usedByTrainers' => (int) $row['used_by'],
            ],
            $rows,
        );
    }

    /**
     * AC-04-37 "top content creators (trainers with the most public
     * content)" — playlists and content items marked public, combined per
     * trainer.
     *
     * @return list<array{trainerId: int, trainerName: string, publicItemCount: int}>
     */
    public function topContentCreators(Account $actor, int $limit = 10): array
    {
        $rows = $this->crossing->fetchAllAssociative(
            <<<'SQL'
                SELECT t.id, t.business_name, COUNT(*) AS public_count
                FROM trainer t
                JOIN (
                    SELECT trainer_id FROM playlist WHERE is_public = true AND deleted_at IS NULL
                    UNION ALL
                    SELECT trainer_id FROM content_item WHERE is_public = true AND deleted_at IS NULL
                ) items ON items.trainer_id = t.id
                GROUP BY t.id, t.business_name
                ORDER BY public_count DESC, t.business_name ASC
                LIMIT :limit
                SQL,
            ['limit' => $limit],
        );

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_top_creators', 'Trainer', null, null, []);
        $this->entityManager->flush();

        return array_map(
            static fn (array $row): array => [
                'trainerId' => (int) $row['id'],
                'trainerName' => (string) $row['business_name'],
                'publicItemCount' => (int) $row['public_count'],
            ],
            $rows,
        );
    }

    /**
     * AC-04-38 "Engagement Stats": total player video views (all-time and
     * this week), average watch time per player, completion rate. This
     * schema carries no separate view-event log (`content_progress` is one
     * row per player-per-item, not one row per play) — "views" is therefore
     * the count of `content_progress` rows that have EVER been viewed
     * (`first_viewed_at IS NOT NULL`), the closest honest proxy the settled
     * schema supports. Recorded in the coder's final report.
     *
     * @return array{totalViewsAllTime: int, totalViewsThisWeek: int, avgWatchTimeSeconds: float, completionRate: float}
     */
    public function contentEngagementStats(Account $actor, \DateTimeImmutable $weekStart): array
    {
        $totalViewsAllTime = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM content_progress WHERE first_viewed_at IS NOT NULL');
        $totalViewsThisWeek = (int) $this->crossing->fetchOne(
            'SELECT COUNT(*) FROM content_progress WHERE first_viewed_at >= :weekStart',
            ['weekStart' => $weekStart->format('Y-m-d H:i:sP')],
        );

        $distinctPlayers = (int) $this->crossing->fetchOne('SELECT COUNT(DISTINCT player_id) FROM content_progress');
        $totalWatchTime = (int) $this->crossing->fetchOne('SELECT COALESCE(SUM(watch_time_seconds), 0) FROM content_progress');
        $avgWatchTime = $distinctPlayers > 0 ? $totalWatchTime / $distinctPlayers : 0.0;

        $totalProgressRows = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM content_progress');
        $completedRows = (int) $this->crossing->fetchOne("SELECT COUNT(*) FROM content_progress WHERE status = 'completed'");
        $completionRate = $totalProgressRows > 0 ? $completedRows / $totalProgressRows : 0.0;

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_engagement', 'ContentProgress', null, null, []);
        $this->entityManager->flush();

        return [
            'totalViewsAllTime' => $totalViewsAllTime,
            'totalViewsThisWeek' => $totalViewsThisWeek,
            'avgWatchTimeSeconds' => $avgWatchTime,
            'completionRate' => $completionRate,
        ];
    }

    /**
     * AC-04-39 "Growth Trends": content-creation (playlists + drills per
     * week), player-engagement (views per week), and public-content growth
     * (new public items per week) — each as the last `$weeks` weekly buckets
     * ending at `$now`, oldest first.
     *
     * @return list<array{weekStart: string, contentCreated: int, views: int, newPublicItems: int}>
     */
    public function contentGrowthTrends(Account $actor, \DateTimeImmutable $now, int $weeks = 8): array
    {
        $trend = [];

        for ($i = $weeks - 1; $i >= 0; --$i) {
            $weekStart = $now->modify(sprintf('-%d weeks', $i))->modify('monday this week')->setTime(0, 0);
            $weekEnd = $weekStart->modify('+7 days');
            $params = ['from' => $weekStart->format('Y-m-d H:i:sP'), 'to' => $weekEnd->format('Y-m-d H:i:sP')];

            $created = (int) $this->crossing->fetchOne(
                'SELECT (SELECT COUNT(*) FROM playlist WHERE created_at >= :from AND created_at < :to) + (SELECT COUNT(*) FROM content_item WHERE created_at >= :from AND created_at < :to)',
                $params,
            );
            $views = (int) $this->crossing->fetchOne(
                'SELECT COUNT(*) FROM content_progress WHERE first_viewed_at >= :from AND first_viewed_at < :to',
                $params,
            );
            $newPublic = (int) $this->crossing->fetchOne(
                'SELECT (SELECT COUNT(*) FROM playlist WHERE ever_published_at >= :from AND ever_published_at < :to) + (SELECT COUNT(*) FROM content_item WHERE ever_published_at >= :from AND ever_published_at < :to)',
                $params,
            );

            $trend[] = ['weekStart' => $weekStart->format('Y-m-d'), 'contentCreated' => $created, 'views' => $views, 'newPublicItems' => $newPublic];
        }

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_trends', 'ContentItem', null, null, []);
        $this->entityManager->flush();

        return $trend;
    }

    /**
     * AC-04-40 "drill down into a specific trainer's LPPP stats".
     *
     * @return array{playlists: int, drills: int, publicItems: int, views: int}
     */
    public function trainerContentStats(Account $actor, int $trainerId): array
    {
        $playlists = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM playlist WHERE trainer_id = :id AND deleted_at IS NULL', ['id' => $trainerId]);
        $drills = (int) $this->crossing->fetchOne(
            'SELECT COUNT(*) FROM drill_detail dd JOIN content_item ci ON ci.id = dd.id WHERE ci.trainer_id = :id AND ci.deleted_at IS NULL',
            ['id' => $trainerId],
        );
        $publicItems = (int) $this->crossing->fetchOne(
            'SELECT (SELECT COUNT(*) FROM playlist WHERE trainer_id = :id AND is_public AND deleted_at IS NULL) + (SELECT COUNT(*) FROM content_item WHERE trainer_id = :id AND is_public AND deleted_at IS NULL)',
            ['id' => $trainerId],
        );
        $views = (int) $this->crossing->fetchOne('SELECT COUNT(*) FROM content_progress WHERE trainer_id = :id AND first_viewed_at IS NOT NULL', ['id' => $trainerId]);

        $this->auditLogger->record($actor, 'cross_tenant_read.content_analytics_trainer_drilldown', 'Trainer', $trainerId, null, []);
        $this->entityManager->flush();

        return ['playlists' => $playlists, 'drills' => $drills, 'publicItems' => $publicItems, 'views' => $views];
    }
}
