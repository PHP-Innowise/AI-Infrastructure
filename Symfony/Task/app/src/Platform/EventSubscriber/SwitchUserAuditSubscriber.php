<?php

declare(strict_types=1);

namespace App\Platform\EventSubscriber;

use App\Identity\Entity\Account;
use App\Platform\Entity\ImpersonationSession;
use App\Platform\Repository\ImpersonationSessionRepository;
use App\Platform\Service\ImpersonationService;
use Symfony\Component\EventDispatcher\EventSubscriberInterface;
use Symfony\Component\Security\Core\Authentication\Token\SwitchUserToken;
use Symfony\Component\Security\Http\Event\SwitchUserEvent;
use Symfony\Component\Security\Http\SecurityEvents;

/**
 * Records both ends of an impersonation — from underneath the mechanism that
 * performs it.
 *
 * `SwitchUserEvent` is dispatched by Symfony's own firewall listener for
 * every identity swap it makes, whichever URL carried the `_switch_user`
 * parameter. Recording here rather than in the controller is the difference
 * between an audit trail and an audit trail with a way around it: manual
 * testing found that a Super Admin could append `?_switch_user=<email>` to
 * any URL and read a parent's payment methods as them, with the audit log
 * count unchanged, because the only place that wrote `impersonation_start`
 * was the one controller a well-behaved admin clicks through.
 *
 * Enter and exit are told apart by the token the event carries, not by the
 * query parameter: entering produces a `SwitchUserToken`, exiting restores
 * the original. A re-switch straight from one target to another dispatches
 * both, in that order, so the first session closes before the second opens.
 *
 * Exit also decides its own reason: re-checking `isExpired()` against the
 * ORIGINAL `startedAt` distinguishes an admin who clicked "Exit
 * Impersonation" from one whose session had already run past the 1-hour mark
 * (AC-01-38), without `ImpersonationExpiryListener` passing that along.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-33..38, AC-01-76, BR-01-21/22
 */
final class SwitchUserAuditSubscriber implements EventSubscriberInterface
{
    public function __construct(
        private readonly ImpersonationSessionRepository $sessions,
        private readonly ImpersonationService $impersonationService,
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
        $token = $event->getToken();

        if ($token instanceof SwitchUserToken) {
            $this->recordStart($token, $event->getTargetUser());

            return;
        }

        $this->recordEnd($event->getTargetUser());
    }

    private function recordStart(SwitchUserToken $token, object $target): void
    {
        $admin = $token->getOriginalToken()->getUser();

        if (!$admin instanceof Account || !$target instanceof Account) {
            return;
        }

        // The firewall has already consulted ImpersonationVoter by this
        // point — security.yaml names IMPERSONATION_START as the switch_user
        // attribute — so reaching here means the swap was permitted. What is
        // left is to make sure it is never silent.
        $this->impersonationService->start($admin, $target);
    }

    private function recordEnd(object $admin): void
    {
        if (!$admin instanceof Account) {
            return;
        }

        $session = $this->sessions->findOpenSessionForAdmin($admin);

        if (null === $session) {
            return;
        }

        $now = new \DateTimeImmutable();
        $this->impersonationService->end(
            $session,
            $session->isExpired($now) ? ImpersonationSession::REASON_EXPIRED : ImpersonationSession::REASON_MANUAL,
            $now,
        );
    }
}
