<?php

declare(strict_types=1);

namespace App\Forms\Repository;

use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<FormSubmission>
 */
class FormSubmissionRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, FormSubmission::class);
    }

    /**
     * BR-08-10: the same email cannot submit the same form twice — the
     * application-level check backed by `uniq_form_submission_form_email`
     * as the final, authoritative guard under concurrency.
     */
    public function findOneByFormAndEmail(Form $form, string $email): ?FormSubmission
    {
        return $this->findOneBy(['form' => $form, 'contactEmail' => $email]);
    }

    /**
     * AC-08-15/BR-08-7: capacity counts CONFIRMED (free|paid) submissions
     * only — a pending payment never holds a spot. Called only while the
     * form row is already locked (`FormRepository::lockForUpdate()`), so
     * this count is safe to act on.
     */
    public function countConfirmedForForm(Form $form): int
    {
        return (int) $this->createQueryBuilder('s')
            ->select('COUNT(s.id)')
            ->andWhere('s.form = :form')
            ->andWhere('s.paymentStatus IN (:statuses)')
            ->setParameter('form', $form)
            ->setParameter('statuses', [FormSubmission::STATUS_FREE, FormSubmission::STATUS_PAID])
            ->getQuery()
            ->getSingleScalarResult();
    }

    /**
     * AC-08-17/18/22: the trainer's participant list — by rule, only
     * paid/free registrations are shown (pending excluded); see
     * `FormSubmissionService::submissionsFor()`'s own docblock for the
     * AC-08-18-vs-Business-Rules conflict this resolves. Newest first, so a
     * long-running camp/evaluation shows recent activity first.
     *
     * @return list<FormSubmission>
     */
    public function findConfirmedForForm(Form $form, ?string $conversionStatus = null): array
    {
        $qb = $this->createQueryBuilder('s')
            ->andWhere('s.form = :form')
            ->andWhere('s.paymentStatus IN (:statuses)')
            ->setParameter('form', $form)
            ->setParameter('statuses', [FormSubmission::STATUS_FREE, FormSubmission::STATUS_PAID])
            ->orderBy('s.submittedAt', 'DESC');

        if ('converted' === $conversionStatus) {
            $qb->andWhere('s.convertedAccount IS NOT NULL');
        } elseif ('not-converted' === $conversionStatus) {
            $qb->andWhere('s.convertedAccount IS NULL');
        }

        /** @var list<FormSubmission> $rows */
        $rows = $qb->getQuery()->getResult();

        return $rows;
    }

    /**
     * AC-08-21: every one of a camp's participants, for the bulk email —
     * same confirmed-only scope as the participant list itself.
     *
     * @return list<FormSubmission>
     */
    public function findAllConfirmedForForm(Form $form): array
    {
        return $this->findConfirmedForForm($form);
    }

    /**
     * AC-08-31/`FormHasSubmissionsException`: EVERY submission regardless of
     * payment status — `form_submission.form_id` is `FK -> form(id)
     * RESTRICT`, so a pending (never-confirmed) registration blocks a hard
     * delete exactly as a paid one does. Deliberately not scoped to
     * confirmed-only, unlike every other query in this class.
     *
     * @return list<FormSubmission>
     */
    public function findAllForForm(Form $form): array
    {
        /** @var list<FormSubmission> $rows */
        $rows = $this->createQueryBuilder('s')
            ->andWhere('s.form = :form')
            ->setParameter('form', $form)
            ->getQuery()
            ->getResult();

        return $rows;
    }

    public function add(FormSubmission $submission): void
    {
        $this->getEntityManager()->persist($submission);
    }
}
