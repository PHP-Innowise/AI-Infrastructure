<?php

declare(strict_types=1);

namespace App\Crm\Repository;

use App\Crm\Entity\Label;
use App\Crm\Entity\PlayerLabel;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Label>
 */
class LabelRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Label::class);
    }

    /**
     * AC-03-11's "Manage Labels" list, and the dropdown of every created
     * label AC-03-12's multi-select applies from.
     *
     * @return list<Label>
     */
    public function findAllForActiveTenant(): array
    {
        /** @var list<Label> $rows */
        $rows = $this->createQueryBuilder('l')
            ->orderBy('l.name', 'ASC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * BR-03-3: the case-insensitive pre-check ("Elite" and "elite" collide)
     * — a nicer UX than waiting for the database's own
     * `uniq_label_trainer_name_normalized` violation, which remains the
     * authoritative backstop under concurrency (see Label's own docblock).
     * $excluding lets an edit skip comparing the label against itself.
     */
    public function findOneByTrainerAndNameCaseInsensitive(Trainer $trainer, string $name, ?Label $excluding = null): ?Label
    {
        $qb = $this->createQueryBuilder('l')
            ->andWhere('IDENTITY(l.trainer) = :trainerId')
            ->andWhere('LOWER(l.name) = LOWER(:name)')
            ->setParameter('trainerId', $trainer->getId())
            ->setParameter('name', $name)
            ->setMaxResults(1);

        if (null !== $excluding) {
            $qb->andWhere('l != :excluding')->setParameter('excluding', $excluding);
        }

        return $qb->getQuery()->getOneOrNullResult();
    }

    /**
     * AC-03-14: "Applied to 12 players."
     */
    public function countPlayers(Label $label): int
    {
        $count = $this->getEntityManager()->createQueryBuilder()
            ->select('COUNT(pl.id)')
            ->from(PlayerLabel::class, 'pl')
            ->andWhere('pl.label = :label')
            ->setParameter('label', $label)
            ->getQuery()
            ->getSingleScalarResult();

        return (int) $count;
    }

    public function add(Label $label): void
    {
        $this->getEntityManager()->persist($label);
    }

    public function remove(Label $label): void
    {
        $this->getEntityManager()->remove($label);
    }
}
