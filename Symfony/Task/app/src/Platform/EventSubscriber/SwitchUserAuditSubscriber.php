<?php

declare(strict_types=1);

namespace App\Platform\EventSubscriber;

use App\Identity\Entity\Account;
use App\Platform\Entity\ImpersonationSession;
use App\Platform\Repository\ImpersonationSessionRepository;
use App\Platform\Service\AuditLogger;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\EventDispatcher\EventSubscriberInterface;
use Symfony\Component\Security\Http\Event\SwitchUserEvent;
use Symfony\Component\Security\Http\SecurityEvents;

/**
 * Closes the ImpersonationSession row Symfony's native `switch_user` exit
 * (`?_switch_user=_exit`) leaves open. AC-01-35 (exit returns the Super Admin
 * to their own view) is switch_user's own job; this class only records that
 * it happened, and — by re-checking `isExpired()` against the ORIGINAL
 * `startedAt` at the moment of exit — distinguishes an admin who clicked
 * "Exit Impersonation" from one whose session had already run past the
 * 1-hour mark (AC-01-38), without ImpersonationExpiryListener needing to
 * pass that fact along separately.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-35, AC-01-36, AC-01-38
 */
final class SwitchUserAuditSubscriber implements EventSubscriberInterface
{
    public function __construct(
        private readonly ImpersonationSessionRepository $sessions,
        private readonly EntityManagerInterface $entityManager,
        private readonly AuditLogger $auditLogger,
    ) {
    }

    /**
     * @return array<string, string>
     */
    public static function getSubscribedEvents(): array
    {
        return [SecurityEvents::SWITCH_USER => 'onSwitchUser'];
    }

    public function onSwitchUser(SwitchUserEvent $event): void
    {
        if ('_exit' !== $event->getRequest()->query->get('_switch_user')) {
            // Entering impersonation, not exiting — the session row is
            // created explicitly by ImpersonationService::start(), called
            // from the controller before the redirect that triggers this
            // very event. Nothing to do here.
            return;
        }

        $admin = $event->getTargetUser();

        if (!$admin instanceof Account) {
            return;
        }

        $session = $this->sessions->findOpenSessionForAdmin($admin);

        if (null === $session) {
            return;
        }

        $now = new \DateTimeImmutable();
        $reason = $session->isExpired($now) ? ImpersonationSession::REASON_EXPIRED : ImpersonationSession::REASON_MANUAL;
        $session->end($reason, $now);

        // AC-01-76: the end of a sensitive operation is logged too, not only
        // its start.
        $this->auditLogger->record(
            $admin,
            'impersonation_end',
            'account',
            $session->getTargetAccount()->getId(),
            null,
            ['reason' => $reason, 'duration_seconds' => $session->durationSeconds()],
        );

        $this->entityManager->flush();
    }
}
