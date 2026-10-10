<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use App\Identity\Entity\ShareLink;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ShareLink>
 */
class ShareLinkRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ShareLink::class);
    }

    /**
     * Reached only once a tenant is already resolved (via PublicTenantCode).
     * RLS scopes this to the active tenant automatically.
     */
    public function findOneByCode(string $code): ?ShareLink
    {
        return $this->findOneBy(['code' => $code]);
    }

    /**
     * AC-01-73: the trainer's static, unlimited-use player link. Provisioned
     * once per trainer; used both for display and as the implicit link when
     * a parent adds a child via "My Trainers" rather than a fresh code.
     */
    public function findStaticPlayerLink(int $trainerId): ?ShareLink
    {
        return $this->createQueryBuilder('s')
            ->andWhere('IDENTITY(s.trainer) = :trainerId')
            ->andWhere('s.linkType = :type')
            ->setParameter('trainerId', $trainerId)
            ->setParameter('type', ShareLink::TYPE_STATIC_PLAYER)
            ->setMaxResults(1)
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * @return list<ShareLink>
     */
    public function findCoachInvitesForTrainer(int $trainerId): array
    {
        /** @var list<ShareLink> $rows */
        $rows = $this->createQueryBuilder('s')
            ->andWhere('IDENTITY(s.trainer) = :trainerId')
            ->andWhere('s.linkType = :type')
            ->setParameter('trainerId', $trainerId)
            ->setParameter('type', ShareLink::TYPE_UNIQUE_COACH)
            ->orderBy('s.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-52: "the coach can view their sent invitations and status
     * (pending, accepted)" — own invites only, not every coach's.
     *
     * @return list<ShareLink>
     */
    public function findCoachPlayerInvitesCreatedBy(Account $account): array
    {
        /** @var list<ShareLink> $rows */
        $rows = $this->createQueryBuilder('s')
            ->andWhere('s.linkType = :type')
            ->andWhere('s.createdByAccount = :account')
            ->setParameter('type', ShareLink::TYPE_COACH_PLAYER_INVITE)
            ->setParameter('account', $account)
            ->orderBy('s.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-03-61/63 (optional MVP): every unique per-player link this trainer
     * has issued, most recent first — the trainer's own tracking report.
     *
     * @return list<ShareLink>
     */
    public function findUniquePlayerInvitesForTrainer(int $trainerId): array
    {
        /** @var list<ShareLink> $rows */
        $rows = $this->createQueryBuilder('s')
            ->andWhere('IDENTITY(s.trainer) = :trainerId')
            ->andWhere('s.linkType = :type')
            ->setParameter('trainerId', $trainerId)
            ->setParameter('type', ShareLink::TYPE_UNIQUE_PLAYER_INVITE)
            ->orderBy('s.createdAt', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(ShareLink $shareLink): void
    {
        $this->getEntityManager()->persist($shareLink);
    }
}
