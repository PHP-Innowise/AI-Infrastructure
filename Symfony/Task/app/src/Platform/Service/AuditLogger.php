<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Identity\Entity\Account;
use App\Platform\Entity\AuditLogEntry;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * The sole writer of AuditLogEntry rows (AC-01-76). A thin adapter: it only
 * persists — the calling service's own flush/transaction commits the entry
 * together with whatever it is logging, so an audit entry can never exist
 * for a write that itself rolled back.
 *
 * Every mutating service that performs a sensitive operation (impersonation,
 * account deactivation/reactivation/GDPR deletion, coach-availability
 * overrides once Epic-02 exists) calls this rather than persisting an
 * AuditLogEntry directly.
 *
 * @see specs/api-designer-spec.md "Impersonation and administrative tenant scope"
 *      — Attributability table: actorAccountId must be the TRUE actor,
 *      recovered from SwitchUserToken::getOriginalToken() when impersonation
 *      is active, never from getUser().
 */
final readonly class AuditLogger
{
    public function __construct(
        private EntityManagerInterface $entityManager,
    ) {
    }

    /**
     * @param array<string, mixed> $details
     */
    public function record(
        ?Account $actor,
        string $actionType,
        string $subjectType,
        ?int $subjectId,
        ?Trainer $relatedTrainer = null,
        array $details = [],
    ): AuditLogEntry {
        $entry = new AuditLogEntry($actor, $actionType, $subjectType, $subjectId, $relatedTrainer, $details);
        $this->entityManager->persist($entry);

        return $entry;
    }
}
