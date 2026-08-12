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
 * job. That voter is now the `switch_user` firewall's own attribute, so it
 * has already ruled by the time either method here is called — from
 * `SwitchUserAuditSubscriber`, which reacts to the swap itself rather than
 * to the one controller that politely asks for it.
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

    /**
     * AC-01-76: the end of a sensitive operation is logged too, not only its
     * start. AC-01-38's automatic expiry and a manual "Exit Impersonation"
     * arrive here identically, distinguished only by $reason.
     */
    public function end(ImpersonationSession $session, string $reason, ?\DateTimeImmutable $at = null): void
    {
        $session->end($reason, $at);

        $this->auditLogger->record(
            $session->getAdminAccount(),
            'impersonation_end',
            'account',
            $session->getTargetAccount()->getId(),
            null,
            ['reason' => $reason, 'duration_seconds' => $session->durationSeconds()],
        );

        $this->entityManager->flush();
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
