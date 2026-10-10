<?php

declare(strict_types=1);

namespace App\Growth\Repository;

use App\Growth\Entity\ReferralAssistCount;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\DBAL\LockMode;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<ReferralAssistCount>
 */
class ReferralAssistCountRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, ReferralAssistCount::class);
    }

    public function findOneByTrainerAndPlayer(Trainer $trainer, PlayerProfile $player): ?ReferralAssistCount
    {
        return $this->findOneBy(['trainer' => $trainer, 'player' => $player]);
    }

    /**
     * Same "insert-then-lock" idiom as
     * `TokenBalanceRepository::lockForUpdate()`: two of the SAME referrer's
     * referees could convert concurrently, both incrementing this exact row
     * — the row lock is what keeps the increment-and-compare-to-threshold
     * sequence in `ReferralRewardService` correct under that race.
     */
    public function lockForUpdate(Trainer $trainer, PlayerProfile $player): ReferralAssistCount
    {
        $connection = $this->getEntityManager()->getConnection();

        $connection->executeStatement(
            'INSERT INTO referral_assist_count (trainer_id, player_id, assist_count, updated_at)
             VALUES (?, ?, 0, ?) ON CONFLICT (trainer_id, player_id) DO NOTHING',
            [$trainer->getId(), $player->getId(), (new \DateTimeImmutable())->format('Y-m-d H:i:sP')],
        );

        /** @var ReferralAssistCount|null $assistCount */
        $assistCount = $this->createQueryBuilder('a')
            ->andWhere('a.trainer = :trainer')
            ->andWhere('a.player = :player')
            ->setParameter('trainer', $trainer)
            ->setParameter('player', $player)
            ->getQuery()
            ->setLockMode(LockMode::PESSIMISTIC_WRITE)
            ->getOneOrNullResult();

        if (null === $assistCount) {
            // Unreachable in practice (the insert above guarantees a row).
            throw new \LogicException('Referral assist count row could not be created or located.');
        }

        return $assistCount;
    }

    public function add(ReferralAssistCount $assistCount): void
    {
        $this->getEntityManager()->persist($assistCount);
    }
}
