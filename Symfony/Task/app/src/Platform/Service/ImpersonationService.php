<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Identity\Entity\Account;
use App\Platform\Entity\ImpersonationSession;
use App\Platform\Repository\ImpersonationSessionRepository;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-01.07: records impersonation sessions. The actual identity swap is
 * Symfony's native `switch_user` firewall feature (recommended vehicle per
 * specs/api-designer-spec.md "Impersonation — a whole session, one identity
 * swap"); this service only owns the audit-bearing row — who impersonated
 * whom, when it started, when (and why) it ended.
 *
 * BR-01-21 ("cannot target another Super Admin") is `ImpersonationVoter`'s
 * job, checked by the controller before this service is ever called — see
 * that voter.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.07, AC-01-33..38, BR-01-21/22
 */
final readonly class ImpersonationService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private ImpersonationSessionRepository $sessions,
        private AuditLogger $auditLogger,
    ) {
    }

    /**
     * AC-01-76: impersonation is a sensitive operation the audit log must
     * capture.
     */
    public function start(Account $admin, Account $target): ImpersonationSession
    {
        $session = new ImpersonationSession($admin, $target);
        $this->sessions->add($session);
        $this->auditLogger->record(
            $admin,
            'impersonation_start',
            'account',
            $target->getId(),
            null,
            ['target_email' => $target->getEmail()],
        );
        $this->entityManager->flush();

        return $session;
    }

    public function findOpenSessionForAdmin(Account $admin): ?ImpersonationSession
    {
        return $this->sessions->findOpenSessionForAdmin($admin);
    }

    /**
     * AC-01-36: the Impersonation History audit report.
     *
     * @return list<ImpersonationSession>
     */
    public function history(): array
    {
        return $this->sessions->findAll();
    }
}
