<?php

declare(strict_types=1);

namespace App\Platform\EventSubscriber;

use App\Identity\Entity\Account;
use App\Platform\Tenancy\TenantContext;
use App\Platform\Tenancy\TenantResolver;
use Symfony\Component\EventDispatcher\EventSubscriberInterface;
use Symfony\Component\HttpKernel\Event\RequestEvent;
use Symfony\Component\HttpKernel\KernelEvents;
use Symfony\Component\Security\Core\Authentication\Token\Storage\TokenStorageInterface;

/**
 * Establishes the tenant once per request, after the firewall has
 * authenticated so the account is known, and before any controller runs.
 *
 * Deliberately a thin adapter: it resolves and activates, and holds no
 * business logic of its own. Priority 6 puts it after Symfony's firewall
 * (priority 8) and router, and before the controller resolver.
 *
 * @see specs/architect-architecture.md "Layer 3 — the mandatory tenant context"
 */
final class TenantRequestSubscriber implements EventSubscriberInterface
{
    public function __construct(
        private readonly TenantResolver $resolver,
        private readonly TenantContext $tenantContext,
        private readonly TokenStorageInterface $tokenStorage,
    ) {
    }

    /**
     * @return array<string, array{0: string, 1: int}>
     */
    public static function getSubscribedEvents(): array
    {
        return [
            KernelEvents::REQUEST => ['onKernelRequest', 6],
        ];
    }

    public function onKernelRequest(RequestEvent $event): void
    {
        if (!$event->isMainRequest()) {
            return;
        }

        $user = $this->tokenStorage->getToken()?->getUser();
        $account = $user instanceof Account ? $user : null;

        $trainerId = $this->resolver->resolve($event->getRequest(), $account);

        if (null === $trainerId) {
            // No source applied. The context stays unresolved on purpose:
            // trainer-scoped access will throw rather than silently read an
            // empty world.
            $this->tenantContext->clear();

            return;
        }

        $this->tenantContext->activate($trainerId);
    }
}
