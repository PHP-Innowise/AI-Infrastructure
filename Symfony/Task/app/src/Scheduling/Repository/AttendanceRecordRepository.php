<?php

declare(strict_types=1);

namespace App\Scheduling\Repository;

use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\AttendanceRecord;
use App\Scheduling\Entity\Event;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<AttendanceRecord>
 */
class AttendanceRecordRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, AttendanceRecord::class);
    }

    public function findOneByEventAndPlayer(Event $event, PlayerProfile $player): ?AttendanceRecord
    {
        return $this->findOneBy(['event' => $event, 'player' => $player]);
    }

    /**
     * @return list<AttendanceRecord>
     */
    public function findForEvent(Event $event): array
    {
        /** @var list<AttendanceRecord> $rows */
        $rows = $this->createQueryBuilder('a')
            ->andWhere('a.event = :event')
            ->setParameter('event', $event)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-31: the player detail's Event History section, most recent
     * event first, within the active tenant.
     *
     * @return list<AttendanceRecord>
     */
    public function findForPlayer(PlayerProfile $player): array
    {
        /** @var list<AttendanceRecord> $rows */
        $rows = $this->createQueryBuilder('a')
            ->addSelect('e')
            ->join('a.event', 'e')
            ->andWhere('a.player = :player')
            ->setParameter('player', $player)
            ->orderBy('e.startsAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-44: a coach-scoped player detail's own attendance history —
     * only sessions THIS coach recorded, not the player's full history with
     * the trainer.
     *
     * @return list<AttendanceRecord>
     */
    public function findForPlayerAndCoach(PlayerProfile $player, CoachMembership $coach): array
    {
        /** @var list<AttendanceRecord> $rows */
        $rows = $this->createQueryBuilder('a')
            ->addSelect('e')
            ->join('a.event', 'e')
            ->andWhere('a.player = :player')
            ->andWhere('a.recordedByCoachMembership = :coach')
            ->setParameter('player', $player)
            ->setParameter('coach', $coach)
            ->orderBy('e.startsAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(AttendanceRecord $record): void
    {
        $this->getEntityManager()->persist($record);
    }

    /**
     * BR-03-16/17: sessions attended (Present or Late only — Absent/Excused
     * never count) in the last 90 days, per player, highest first, ties
     * broken alphabetically by player name (BR-03-16). A player with zero
     * qualifying sessions in the window is never in the result set at all
     * (BR-03-19) — the INNER JOIN plus WHERE means no row is ever produced
     * for them, nothing to filter out afterward.
     *
     * Native SQL over the `(trainer_id, player_id, status, recorded_at)`
     * index, explicitly permitted for this exact query —
     * specs/database-designer-schema.md "Doctrine mapping notes... Top
     * Players... a single aggregate query... explicitly permitted to use
     * native SQL... because RLS covers native SQL." `$trainerId` is passed
     * explicitly as a defense-in-depth predicate: RLS protects native SQL at
     * the database session level, but the Doctrine filter (layer 2) does not
     * rewrite raw SQL the way it rewrites DQL, so this repository states the
     * tenant boundary itself rather than relying on RLS alone — proven by a
     * cross-tenant test (a foreign trainer's players never appear).
     *
     * @return list<array{playerId: int, name: string, sessionCount: int}>
     */
    public function topPlayers(int $trainerId, int $limit = 10, ?\DateTimeImmutable $now = null): array
    {
        $since = ($now ?? new \DateTimeImmutable())->modify('-90 days');

        $rows = $this->getEntityManager()->getConnection()->fetchAllAssociative(
            <<<'SQL'
                SELECT p.id AS player_id, p.first_name AS name, COUNT(a.id) AS session_count
                FROM attendance_record a
                JOIN player_profile p ON p.id = a.player_id
                WHERE a.trainer_id = :trainerId
                  AND a.status IN ('present', 'late')
                  AND a.recorded_at >= :since
                GROUP BY p.id, p.first_name
                ORDER BY session_count DESC, p.first_name ASC
                LIMIT :limit
                SQL,
            [
                'trainerId' => $trainerId,
                'since' => $since->format('Y-m-d H:i:sP'),
                'limit' => $limit,
            ],
        );

        return array_map(
            static fn (array $row): array => [
                'playerId' => (int) $row['player_id'],
                'name' => (string) $row['name'],
                'sessionCount' => (int) $row['session_count'],
            ],
            $rows,
        );
    }

    /**
     * AC-03-40 (optional MVP): average attendance rate and no-show rate
     * inputs — the caller (QuickViewDashboardService) divides. Present/Late
     * count as attended, matching BR-03-17's Top-Players definition exactly
     * rather than inventing a second one; the denominator is every
     * `attendance_record` in the window, mirroring
     * `PlayerSegmentationRepository`'s own attendance-rate definition.
     * `$trainerId` is explicit defense-in-depth, matching `topPlayers()`'s
     * own reasoning for native SQL.
     *
     * @return array{attended: int, absent: int, total: int}
     */
    public function attendanceTallyBetween(int $trainerId, \DateTimeImmutable $from, \DateTimeImmutable $to): array
    {
        $row = $this->getEntityManager()->getConnection()->fetchAssociative(
            <<<'SQL'
                SELECT
                    COUNT(*) FILTER (WHERE status IN ('present','late')) AS attended,
                    COUNT(*) FILTER (WHERE status = 'absent') AS absent,
                    COUNT(*) AS total
                FROM attendance_record
                WHERE trainer_id = :trainerId AND recorded_at >= :from AND recorded_at < :to
                SQL,
            ['trainerId' => $trainerId, 'from' => $from->format('Y-m-d H:i:sP'), 'to' => $to->format('Y-m-d H:i:sP')],
        );

        return [
            'attended' => (int) ($row['attended'] ?? 0),
            'absent' => (int) ($row['absent'] ?? 0),
            'total' => (int) ($row['total'] ?? 0),
        ];
    }
}
