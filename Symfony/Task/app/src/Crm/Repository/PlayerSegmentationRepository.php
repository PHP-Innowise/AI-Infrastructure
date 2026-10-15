<?php

declare(strict_types=1);

namespace App\Crm\Repository;

use App\Crm\Dto\PlayerSummary;
use App\Crm\Dto\SegmentCriteria;
use Doctrine\DBAL\ArrayParameterType;
use Doctrine\ORM\EntityManagerInterface;

/**
 * The AND-combined (BR-03-13) player list/segmentation query — the
 * aggregate-heaviest read in the product (task brief). Native SQL, never
 * hydrates full `PlayerProfile`/`PlayerTrainerMembership` graphs for a
 * screen that shows a row of badges and numbers, matching
 * `specs/database-designer-schema.md` "Doctrine mapping notes... CRM
 * segmentation... PlayerSegmentationRepository::search()... native SQL,
 * never hydrates full PlayerProfile graphs."
 *
 * **Tenancy.** `$trainerId` is bound explicitly into the WHERE clause on
 * every query here, in addition to RLS. RLS alone protects native SQL at
 * the database session level (no Doctrine filter rewrite applies to raw
 * SQL), so the explicit predicate is defense in depth, not redundancy —
 * proven by `PlayerSegmentationTest`'s cross-tenant case: a foreign
 * trainer's players must never appear, tenant-context tricks or not.
 *
 * **Attendance rate.** Defined identically here and on the player detail
 * view (AC-03-31's "18 of 20 events (90%)"): `attended / total tracked
 * events * 100`, where "attended" is Present+Late `attendance_record` rows
 * and "total tracked" is every `attendance_record` row regardless of status
 * — NOT RSVP count. The epic never states the denominator precisely; using
 * the same definition everywhere avoids the "two competing definitions of
 * one fact" drift `specs/database-designer-schema.md`'s own Decisions table
 * repeatedly warns against (e.g. "RSVP status vocabulary"). Recorded as an
 * inference in the coder's final report.
 *
 * **Last activity.** Not a stored column anywhere in the settled schema
 * (`player_trainer_membership` carries no such field) despite Crm's Data
 * Requirements naming one conceptually — derived at read time as the latest
 * of: last attendance record, last RSVP, or the membership's own join date
 * (a floor, so a brand-new player with no activity yet still has a lawful
 * "last activity" rather than NULL forever). Another inference, recorded in
 * the coder's final report.
 *
 * **Labels/flags are OR-within-category.** "Labels (one or more)" (AC-03-22)
 * matches a player holding ANY of the selected labels, not all of them;
 * BR-03-13's AND-combination is across categories (Skill AND Attendance AND
 * Label), not within the Labels category itself — the conventional reading
 * of a multi-select filter, and the one this repository implements.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-1..10, AC-03-22..26, BR-03-13/14
 */
final readonly class PlayerSegmentationRepository
{
    public function __construct(
        private EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * @return array{items: list<PlayerSummary>, total: int}
     */
    public function search(int $trainerId, SegmentCriteria $criteria, ?\DateTimeImmutable $now = null): array
    {
        $now ??= new \DateTimeImmutable();
        $connection = $this->entityManager->getConnection();

        $innerConditions = ['ptm.trainer_id = :trainerId', "ptm.status = 'active'"];
        $params = ['trainerId' => $trainerId];
        /** @var array<string, ArrayParameterType> $types */
        $types = [];

        if (null !== $criteria->query && '' !== trim($criteria->query)) {
            $innerConditions[] = <<<'SQL'
                (
                    p.first_name ILIKE :q
                    OR p.school_or_team ILIKE :q
                    OR EXISTS (
                        SELECT 1 FROM parent_child_link pcl
                        JOIN account_profile ap ON ap.account_id = pcl.parent_account_id
                        JOIN account acc ON acc.id = pcl.parent_account_id
                        WHERE pcl.child_player_id = p.id
                          AND (ap.first_name ILIKE :q OR ap.last_name ILIKE :q OR acc.email::text ILIKE :q)
                    )
                    OR (
                        p.self_account_id IS NOT NULL
                        AND EXISTS (
                            SELECT 1 FROM account_profile ap2
                            JOIN account acc2 ON acc2.id = ap2.account_id
                            WHERE ap2.account_id = p.self_account_id
                              AND (ap2.first_name ILIKE :q OR ap2.last_name ILIKE :q OR acc2.email::text ILIKE :q)
                        )
                    )
                )
                SQL;
            $params['q'] = '%'.$criteria->query.'%';
        }

        if (null !== $criteria->skillLevel && '' !== $criteria->skillLevel) {
            // Compared case-insensitively: the profile field is a choice list
            // now, but rows written while it was free text still hold
            // whatever a trainer typed, and "intermediate" is plainly the
            // same answer as "Intermediate" — see SkillLevel.
            $innerConditions[] = 'lower(btrim(ptm.skill_level)) = lower(btrim(:skillLevel))';
            $params['skillLevel'] = $criteria->skillLevel;
        }

        if (null !== $criteria->minAge) {
            $innerConditions[] = "DATE_PART('year', AGE(CURRENT_DATE, p.date_of_birth)) >= :minAge";
            $params['minAge'] = $criteria->minAge;
        }

        if (null !== $criteria->maxAge) {
            $innerConditions[] = "DATE_PART('year', AGE(CURRENT_DATE, p.date_of_birth)) <= :maxAge";
            $params['maxAge'] = $criteria->maxAge;
        }

        if (null !== $criteria->gender && '' !== $criteria->gender) {
            $innerConditions[] = 'p.gender = :gender';
            $params['gender'] = $criteria->gender;
        }

        if (null !== $criteria->teamSchoolClub && '' !== $criteria->teamSchoolClub) {
            $innerConditions[] = 'p.school_or_team ILIKE :teamSchoolClub';
            $params['teamSchoolClub'] = '%'.$criteria->teamSchoolClub.'%';
        }

        if (null !== $criteria->labelIds && [] !== $criteria->labelIds) {
            $innerConditions[] = 'EXISTS (SELECT 1 FROM player_label pl WHERE pl.player_id = p.id AND pl.label_id IN (:labelIds))';
            $params['labelIds'] = $criteria->labelIds;
            $types['labelIds'] = ArrayParameterType::INTEGER;
        }

        if (null !== $criteria->flagTypes && [] !== $criteria->flagTypes) {
            $innerConditions[] = "EXISTS (SELECT 1 FROM player_flag pf WHERE pf.player_id = p.id AND pf.status = 'active' AND pf.flag_type IN (:flagTypes))";
            $params['flagTypes'] = $criteria->flagTypes;
            $types['flagTypes'] = ArrayParameterType::STRING;
        }

        if (null !== $criteria->registeredFrom) {
            $innerConditions[] = 'ptm.joined_at >= :registeredFrom';
            $params['registeredFrom'] = $criteria->registeredFrom->format('Y-m-d H:i:sP');
        }

        if (null !== $criteria->registeredTo) {
            $innerConditions[] = 'ptm.joined_at <= :registeredTo';
            $params['registeredTo'] = $criteria->registeredTo->format('Y-m-d H:i:sP');
        }

        $params['recentSince90'] = $now->modify('-90 days')->format('Y-m-d H:i:sP');
        if (null !== $criteria->attendedInLastDays) {
            $params['attendedInLastDaysSince'] = $now->modify(sprintf('-%d days', $criteria->attendedInLastDays))->format('Y-m-d H:i:sP');
        }

        $innerWhere = implode(' AND ', $innerConditions);

        $outerConditions = [];

        if (null !== $criteria->attendedMoreThan) {
            $outerConditions[] = 'attended_count > :attendedMoreThan';
            $params['attendedMoreThan'] = $criteria->attendedMoreThan;
        }

        if (null !== $criteria->attendedInLastDays) {
            $outerConditions[] = 'recent_attended_count > 0';
        }

        if (null !== $criteria->attendanceRateGreaterThan) {
            $outerConditions[] = 'attendance_rate > :attendanceRateGreaterThan';
            $params['attendanceRateGreaterThan'] = $criteria->attendanceRateGreaterThan;
        }

        if (null !== $criteria->noShowsGreaterThan) {
            $outerConditions[] = 'no_show_count > :noShowsGreaterThan';
            $params['noShowsGreaterThan'] = $criteria->noShowsGreaterThan;
        }

        if (null !== $criteria->lastActivityBand) {
            $outerConditions[] = match ($criteria->lastActivityBand) {
                SegmentCriteria::BAND_ACTIVE => 'last_activity_at >= :activeSince',
                SegmentCriteria::BAND_INACTIVE => 'last_activity_at < :activeSince AND last_activity_at >= :churnedSince',
                SegmentCriteria::BAND_CHURNED => 'last_activity_at < :churnedSince',
                default => '1=1',
            };
            $params['activeSince'] = $now->modify('-30 days')->format('Y-m-d H:i:sP');
            $params['churnedSince'] = $now->modify('-90 days')->format('Y-m-d H:i:sP');
        }

        $outerWhere = [] === $outerConditions ? '1=1' : implode(' AND ', $outerConditions);
        $orderBy = match ($criteria->sort) {
            SegmentCriteria::SORT_NAME_DESC => 'first_name DESC',
            SegmentCriteria::SORT_LAST_ACTIVITY => 'last_activity_at DESC',
            SegmentCriteria::SORT_ATTENDANCE_RATE => 'attendance_rate DESC',
            default => 'first_name ASC',
        };

        $offset = max(0, $criteria->page - 1) * $criteria->perPage;

        $sql = <<<SQL
            WITH attendance_stats AS (
                SELECT
                    a.player_id,
                    COUNT(*) FILTER (WHERE a.status IN ('present','late')) AS attended_count,
                    COUNT(*) AS total_count,
                    COUNT(*) FILTER (WHERE a.status = 'absent') AS no_show_count,
                    MAX(a.recorded_at) AS last_attendance_at,
                    COUNT(*) FILTER (WHERE a.status IN ('present','late') AND a.recorded_at >= :recentSince90) AS recent_attended_count90
                    {$this->recentAttendedSelectFragment($criteria)}
                FROM attendance_record a
                WHERE a.trainer_id = :trainerId
                GROUP BY a.player_id
            ),
            rsvp_stats AS (
                SELECT r.player_id, MAX(r.requested_at) AS last_rsvp_at
                FROM rsvp r
                WHERE r.trainer_id = :trainerId
                GROUP BY r.player_id
            ),
            player_rows AS (
                SELECT
                    ptm.id AS membership_id,
                    p.id AS player_id,
                    p.first_name,
                    p.date_of_birth,
                    p.gender,
                    p.school_or_team,
                    p.photo_url,
                    ptm.skill_level,
                    ptm.joined_at AS registered_at,
                    COALESCE(ast.attended_count, 0) AS attended_count,
                    COALESCE(ast.total_count, 0) AS total_count,
                    COALESCE(ast.no_show_count, 0) AS no_show_count,
                    {$this->recentAttendedOuterFragment($criteria)} AS recent_attended_count,
                    GREATEST(ptm.joined_at, COALESCE(ast.last_attendance_at, ptm.joined_at), COALESCE(rst.last_rsvp_at, ptm.joined_at)) AS last_activity_at,
                    CASE WHEN COALESCE(ast.total_count, 0) = 0 THEN 0
                         ELSE ROUND(100.0 * ast.attended_count / ast.total_count, 1)
                    END AS attendance_rate
                FROM player_trainer_membership ptm
                JOIN player_profile p ON p.id = ptm.player_profile_id
                LEFT JOIN attendance_stats ast ON ast.player_id = p.id
                LEFT JOIN rsvp_stats rst ON rst.player_id = p.id
                WHERE {$innerWhere}
            )
            SELECT * FROM player_rows WHERE {$outerWhere} ORDER BY {$orderBy} LIMIT :limit OFFSET :offset
            SQL;

        $countSql = <<<SQL
            WITH attendance_stats AS (
                SELECT
                    a.player_id,
                    COUNT(*) FILTER (WHERE a.status IN ('present','late')) AS attended_count,
                    COUNT(*) AS total_count,
                    COUNT(*) FILTER (WHERE a.status = 'absent') AS no_show_count,
                    MAX(a.recorded_at) AS last_attendance_at,
                    COUNT(*) FILTER (WHERE a.status IN ('present','late') AND a.recorded_at >= :recentSince90) AS recent_attended_count90
                    {$this->recentAttendedSelectFragment($criteria)}
                FROM attendance_record a
                WHERE a.trainer_id = :trainerId
                GROUP BY a.player_id
            ),
            rsvp_stats AS (
                SELECT r.player_id, MAX(r.requested_at) AS last_rsvp_at
                FROM rsvp r
                WHERE r.trainer_id = :trainerId
                GROUP BY r.player_id
            ),
            player_rows AS (
                SELECT
                    COALESCE(ast.attended_count, 0) AS attended_count,
                    COALESCE(ast.total_count, 0) AS total_count,
                    COALESCE(ast.no_show_count, 0) AS no_show_count,
                    {$this->recentAttendedOuterFragment($criteria)} AS recent_attended_count,
                    GREATEST(ptm.joined_at, COALESCE(ast.last_attendance_at, ptm.joined_at), COALESCE(rst.last_rsvp_at, ptm.joined_at)) AS last_activity_at,
                    CASE WHEN COALESCE(ast.total_count, 0) = 0 THEN 0
                         ELSE ROUND(100.0 * ast.attended_count / ast.total_count, 1)
                    END AS attendance_rate
                FROM player_trainer_membership ptm
                JOIN player_profile p ON p.id = ptm.player_profile_id
                LEFT JOIN attendance_stats ast ON ast.player_id = p.id
                LEFT JOIN rsvp_stats rst ON rst.player_id = p.id
                WHERE {$innerWhere}
            )
            SELECT COUNT(*) FROM player_rows WHERE {$outerWhere}
            SQL;

        $rows = $connection->fetchAllAssociative($sql, [...$params, 'limit' => $criteria->perPage, 'offset' => $offset], $types);
        $total = (int) $connection->fetchOne($countSql, $params, $types);

        $items = array_map(function (array $row) use ($now): PlayerSummary {
            $dob = new \DateTimeImmutable((string) $row['date_of_birth']);

            return new PlayerSummary(
                membershipId: (int) $row['membership_id'],
                playerId: (int) $row['player_id'],
                name: (string) $row['first_name'],
                photoUrl: null !== $row['photo_url'] ? (string) $row['photo_url'] : null,
                age: $dob->diff($now)->y,
                gender: null !== $row['gender'] ? (string) $row['gender'] : null,
                skillLevel: null !== $row['skill_level'] ? (string) $row['skill_level'] : null,
                teamSchoolClub: null !== $row['school_or_team'] ? (string) $row['school_or_team'] : null,
                attendedCount: (int) $row['attended_count'],
                totalTrackedCount: (int) $row['total_count'],
                noShowCount: (int) $row['no_show_count'],
                attendanceRate: (float) $row['attendance_rate'],
                registeredAt: new \DateTimeImmutable((string) $row['registered_at']),
                lastActivityAt: new \DateTimeImmutable((string) $row['last_activity_at']),
            );
        }, $rows);

        return ['items' => $items, 'total' => $total];
    }

    /**
     * The "attended in last Y days" CTE column only needs to exist when that
     * filter is active — Y is caller-supplied, not a fixed window, so it
     * cannot be precomputed as part of the always-present 90-day column
     * `topPlayers()`-style. Kept as a conditionally-included fragment rather
     * than always joining a second, possibly-redundant aggregate.
     */
    private function recentAttendedSelectFragment(SegmentCriteria $criteria): string
    {
        if (null === $criteria->attendedInLastDays) {
            return '';
        }

        return ", COUNT(*) FILTER (WHERE a.status IN ('present','late') AND a.recorded_at >= :attendedInLastDaysSince) AS recent_attended_count_y";
    }

    private function recentAttendedOuterFragment(SegmentCriteria $criteria): string
    {
        return null === $criteria->attendedInLastDays
            ? 'COALESCE(ast.recent_attended_count90, 0)'
            : 'COALESCE(ast.recent_attended_count_y, 0)';
    }
}
