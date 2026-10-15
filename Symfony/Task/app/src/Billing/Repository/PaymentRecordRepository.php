<?php

declare(strict_types=1);

namespace App\Billing\Repository;

use App\Billing\Entity\PaymentRecord;
use App\Content\Entity\Playlist;
use App\Identity\Entity\Account;
use App\Scheduling\Entity\Rsvp;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<PaymentRecord>
 */
class PaymentRecordRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, PaymentRecord::class);
    }

    public function findOneByStripePaymentIntentId(string $stripePaymentIntentId): ?PaymentRecord
    {
        return $this->findOneBy(['stripePaymentIntentId' => $stripePaymentIntentId]);
    }

    public function findOneByStripeChargeId(string $stripeChargeId): ?PaymentRecord
    {
        return $this->findOneBy(['stripeChargeId' => $stripeChargeId]);
    }

    public function findOneByStripeRefundId(string $stripeRefundId): ?PaymentRecord
    {
        return $this->findOneBy(['stripeRefundId' => $stripeRefundId]);
    }

    /**
     * The charge row for one RSVP — the ORIGINAL charge, never a refund
     * (`refundsPaymentRecord IS NULL`), since a refund row shares the same
     * `relatedRsvp` (PaymentRecord::forRefund() copies it) and would
     * otherwise make this lookup ambiguous.
     */
    public function findOneChargeByRsvp(Rsvp $rsvp): ?PaymentRecord
    {
        return $this->createQueryBuilder('p')
            ->andWhere('p.relatedRsvp = :rsvp')
            ->andWhere('p.refundsPaymentRecord IS NULL')
            ->setParameter('rsvp', $rsvp)
            ->getQuery()
            ->getOneOrNullResult();
    }

    public function findOneChargeByPlaylistAndPlayerAccount(Playlist $playlist, Account $payerAccount): ?PaymentRecord
    {
        return $this->createQueryBuilder('p')
            ->andWhere('p.relatedPlaylist = :playlist')
            ->andWhere('p.payerAccount = :payer')
            ->andWhere('p.refundsPaymentRecord IS NULL')
            ->setParameter('playlist', $playlist)
            ->setParameter('payer', $payerAccount)
            ->orderBy('p.createdAt', 'DESC')
            ->setMaxResults(1)
            ->getQuery()
            ->getOneOrNullResult();
    }

    /**
     * BR-05-9: pending USD payments (bank-transfer-style methods that take
     * time to clear) auto-cancel after 7 days with no confirming webhook.
     *
     * @return list<PaymentRecord>
     */
    public function findPendingOlderThan(\DateTimeImmutable $cutoff): array
    {
        /** @var list<PaymentRecord> $rows */
        $rows = $this->createQueryBuilder('p')
            ->andWhere('p.status = :pending')
            ->andWhere('p.createdAt < :cutoff')
            ->setParameter('pending', PaymentRecord::STATUS_PENDING)
            ->setParameter('cutoff', $cutoff)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * AC-05-22/23, BR-05-17: transaction history, strictly scoped to the
     * current trainer context AND the acting payer account — never another
     * account's rows, even within the same trainer.
     *
     * @param list<string>|null $types
     *
     * @return list<PaymentRecord>
     */
    public function findForPayerHistory(
        Account $payerAccount,
        ?\DateTimeImmutable $from = null,
        ?\DateTimeImmutable $to = null,
        ?array $types = null,
        ?string $paymentMethod = null,
    ): array {
        $qb = $this->createQueryBuilder('p')
            ->andWhere('p.payerAccount = :payer')
            ->setParameter('payer', $payerAccount)
            ->orderBy('p.createdAt', 'DESC');

        if (null !== $from) {
            $qb->andWhere('p.createdAt >= :from')->setParameter('from', $from);
        }

        if (null !== $to) {
            $qb->andWhere('p.createdAt <= :to')->setParameter('to', $to);
        }

        if (null !== $types && [] !== $types) {
            $qb->andWhere('p.type IN (:types)')->setParameter('types', $types);
        }

        if (null !== $paymentMethod) {
            $qb->andWhere('p.paymentMethod = :method')->setParameter('method', $paymentMethod);
        }

        /** @var list<PaymentRecord> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    public function add(PaymentRecord $paymentRecord): void
    {
        $this->getEntityManager()->persist($paymentRecord);
    }
}
