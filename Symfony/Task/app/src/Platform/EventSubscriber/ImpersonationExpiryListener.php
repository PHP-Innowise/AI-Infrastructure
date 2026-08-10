<?php

declare(strict_types=1);

namespace App\Platform\EventSubscriber;

use App\Identity\Entity\Account;
use App\Platform\Repository\ImpersonationSessionRepository;
use Symfony\Component\EventDispatcher\EventSubscriberInterface;
use Symfony\Component\HttpFoundation\RedirectResponse;
use Symfony\Component\HttpKernel\Event\RequestEvent;
use Symfony\Component\HttpKernel\KernelEvents;
use Symfony\Component\Security\Core\Authentication\Token\Storage\TokenStorageInterface;
use Symfony\Component\Security\Core\Authentication\Token\SwitchUserToken;

/**
 * AC-01-38, BR-01-22: "an impersonation session expires automatically after 1
 * hour if not explicitly exited" — enforced at read time (the next request),
 * never by a scheduled job, per architecture "Time-based state is derived at
 * read time".
 *
 * On an expired session, this redirects through Symfony's own
 * `?_switch_user=_exit` mechanism rather than swapping the token by hand, so
 * `SwitchUserAuditSubscriber` records the closure in exactly one place
 * regardless of whether the exit was manual or forced.
 *
 * Priority 7: after the firewall (8, so a token exists to inspect) and before
 * TenantRequestSubscriber (6, so a forced exit's redirect is decided before
 * tenant resolution runs against a stale impersonated identity).
 */
final class ImpersonationExpiryListener implements EventSubscriberInterface
{
    public function __construct(
        private readonly TokenStorageInterface $tokenStorage,
        private readonly ImpersonationSessionRepository $sessions,
    ) {
    }

    /**
     * @return array<string, array{0: string, 1: int}>
     */
    public static function getSubscribedEvents(): array
    {
        return [
            KernelEvents::REQUEST => ['onKernelRequest', 7],
        ];
    }

    public function onKernelRequest(RequestEvent $event): void
    {
        if (!$event->isMainRequest()) {
            return;
        }

        $token = $this->tokenStorage->getToken();

        if (!$token instanceof SwitchUserToken) {
            return;
        }

        $admin = $token->getOriginalToken()->getUser();

        if (!$admin instanceof Account) {
            return;
        }

        $session = $this->sessions->findOpenSessionForAdmin($admin);

        if (null === $session || !$session->isExpired(new \DateTimeImmutable())) {
            return;
        }

        $request = $event->getRequest();
        $separator = str_contains($request->getRequestUri(), '?') ? '&' : '?';
        $exitUrl = $request->getRequestUri().$separator.'_switch_user=_exit';

        $event->setResponse(new RedirectResponse($exitUrl));
    }
}
