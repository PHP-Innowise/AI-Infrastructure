<?php

declare(strict_types=1);

namespace App\Crm\Repository;

use App\Crm\Entity\PlayerFlag;
use App\Identity\Entity\PlayerProfile;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PlayerFlag>
 */
class PlayerFlagRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PlayerFlag::class);
    }

    /**
     * AC-03-16: the player list's flag badges and the player detail's active
     * flag list.
     *
     * @return list<PlayerFlag>
     */
    public function findActiveForPlayer(PlayerProfile $player): array
    {
        /** @var list<PlayerFlag> $rows */
        $rows = $this->createQueryBuilder('f')
            ->andWhere('f.player = :player')
            ->andWhere('f.status = :active')
            ->setParameter('player', $player)
            ->setParameter('active', PlayerFlag::STATUS_ACTIVE)
            ->orderBy('f.appliedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * Full history (active + resolved), most recent first — AC-03-17's "its
     * history is preserved in an audit log."
     *
     * @return list<PlayerFlag>
     */
    public function findAllForPlayer(PlayerProfile $player): array
    {
        /** @var list<PlayerFlag> $rows */
        $rows = $this->createQueryBuilder('f')
            ->andWhere('f.player = :player')
            ->setParameter('player', $player)
            ->orderBy('f.appliedAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * BR-03-8's pre-check: "reapply the same flag later" is legitimate only
     * once the earlier instance is resolved — this is the lookup that
     * detects an already-active duplicate before the database's own
     * `uniq_player_flag_active_type` partial unique index (the authoritative
     * backstop under concurrency) would reject it.
     */
    public function findOneActiveByPlayerAndType(PlayerProfile $player, string $flagType): ?PlayerFlag
    {
        return $this->findOneBy(['player' => $player, 'flagType' => $flagType, 'status' => PlayerFlag::STATUS_ACTIVE]);
    }

    /**
     * AC-03-38: the Quick View "Flag Alerts" widget — active-flag counts per
     * type, for the active tenant.
     *
     * @return array<string, int> flagType => count
     */
    public function countActiveByTypeForActiveTenant(): array
    {
        /** @var list<array{flagType: string, cnt: string}> $rows */
        $rows = $this->createQueryBuilder('f')
            ->select('f.flagType AS flagType', 'COUNT(f.id) AS cnt')
            ->andWhere('f.status = :active')
            ->setParameter('active', PlayerFlag::STATUS_ACTIVE)
            ->groupBy('f.flagType')
            ->getQuery()
            ->getResult();

        $counts = array_fill_keys(PlayerFlag::types(), 0);

        foreach ($rows as $row) {
            $counts[$row['flagType']] = (int) $row['cnt'];
        }

        return $counts;
    }

    public function add(PlayerFlag $flag): void
    {
        $this->getEntityManager()->persist($flag);
    }
}
