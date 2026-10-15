<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountStatus;
use App\Identity\Entity\UserDeletionRecord;
use App\Identity\Repository\UserDeletionRecordRepository;
use App\Platform\Service\AuditLogger;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-01.12/US-01.13: deactivate, reactivate, and GDPR anonymise-in-place.
 * Every operation here is Super-Admin-only (enforced by `AccountVoter` at the
 * controller boundary) and audit-logged.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.12, US-01.13, AC-01-52..59, BR-01-23/24
 */
final readonly class AccountLifecycleService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private UserDeletionRecordRepository $deletionRecords,
        private AuditLogger $auditLogger,
        private UserPasswordHasherInterface $passwordHasher,
    ) {
    }

    /**
     * AC-01-52/53: login is blocked; every historical record stays visible
     * because nothing referencing this account is touched.
     */
    public function deactivate(Account $actingSuperAdmin, Account $target): void
    {
        $this->entityManager->wrapInTransaction(function () use ($actingSuperAdmin, $target): void {
            $target->deactivate();
            $this->auditLogger->record($actingSuperAdmin, 'user_deactivated', 'account', $target->getId(), null);
        });
    }

    /**
     * AC-01-54. Only ever reachable for a merely-deactivated account — a
     * Deleted account cannot reach this method at all (AC-01-58), enforced by
     * `AccountVoter::ACCOUNT_REACTIVATE` denying a Deleted subject.
     */
    public function reactivate(Account $actingSuperAdmin, Account $target): void
    {
        $this->entityManager->wrapInTransaction(function () use ($actingSuperAdmin, $target): void {
            $target->reactivate();
            $this->auditLogger->record($actingSuperAdmin, 'user_reactivated', 'account', $target->getId(), null);
        });
    }

    /**
     * AC-01-55..59: GDPR anonymisation. Permanent — there is no un-delete,
     * because this method does not merely flip a status, it destroys the
     * personal data a reactivation would need to restore.
     */
    public function anonymize(Account $actingSuperAdmin, Account $target, ?string $reason = null): void
    {
        $this->entityManager->wrapInTransaction(function () use ($actingSuperAdmin, $target, $reason): void {
            $originalId = (int) $target->getId();
            $originalEmail = $target->getEmail();

            $this->deletionRecords->add(new UserDeletionRecord($target, $originalEmail, $actingSuperAdmin, $reason));

            // Deterministic replacement email: the UNIQUE constraint can
            // never collide, and any later lookup by the pre-deletion email
            // correctly finds nothing.
            $target->anonymizeEmail(sprintf('deleted_%d@example.com', $originalId));
            // A random, unusable password — login stays blocked even though
            // status transitions are otherwise unrelated to credentials.
            $target->changePasswordHash($this->passwordHasher->hashPassword($target, bin2hex(random_bytes(32))));
            $target->markDeleted();
            $target->getProfile()?->anonymize();

            $this->auditLogger->record(
                $actingSuperAdmin,
                'user_deleted',
                'account',
                $originalId,
                null,
                ['original_email' => $originalEmail, 'reason' => $reason],
            );
        });
    }
}
