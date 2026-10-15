<?php

declare(strict_types=1);

namespace App\Forms\Repository;

use App\Forms\Entity\Form;
use App\Platform\Entity\Trainer;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\DBAL\LockMode;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Form>
 */
class FormRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Form::class);
    }

    /**
     * Reached only once a tenant is already resolved (via PublicTenantCode,
     * on an allow-listed route) — RLS scopes this to the active tenant
     * automatically, matching `ShareLinkRepository::findOneByCode()`'s own
     * precedent exactly.
     */
    public function findOneBySlug(string $slug): ?Form
    {
        return $this->findOneBy(['shareableSlug' => $slug]);
    }

    /**
     * AC-08-27: the trainer's own list of all camps and evaluations, newest
     * first. `created_at` is `TIMESTAMP(0)` (second precision, per this
     * table's migration) — two forms created within the same second sort
     * as ties on that column alone, so `id DESC` breaks the tie
     * deterministically (a higher, IDENTITY-assigned id is always more
     * recent).
     *
     * @return list<Form>
     */
    public function findAllForTrainer(Trainer $trainer): array
    {
        /** @var list<Form> $rows */
        $rows = $this->createQueryBuilder('f')
            ->andWhere('f.trainer = :trainer')
            ->setParameter('trainer', $trainer)
            ->orderBy('f.createdAt', 'DESC')
            ->addOrderBy('f.id', 'DESC')
            ->getQuery()
            ->getResult();

        return $rows;
    }

    /**
     * "Risks & Mitigations — Capacity Overselling": "atomic, database-level
     * check... inside the submission transaction" — locks the form row so
     * concurrent submissions serialize against the capacity count that
     * follows.
     */
    public function lockForUpdate(int $formId): ?Form
    {
        /** @var Form|null $form */
        $form = $this->getEntityManager()->find(Form::class, $formId, LockMode::PESSIMISTIC_WRITE);

        return $form;
    }

    public function add(Form $form): void
    {
        $this->getEntityManager()->persist($form);
    }

    public function remove(Form $form): void
    {
        $this->getEntityManager()->remove($form);
    }
}
