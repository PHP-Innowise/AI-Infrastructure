<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Repository\PublicTenantCodeRepository;
use Symfony\Component\HttpFoundation\Request;

/**
 * Determines the tenant for a request. The only setter of TenantContext
 * outside an administrative scope.
 *
 * Resolution sources, in the architecture's order. Sources 1 and 2
 * (administrative scope, impersonation) arrive with Epic-07; this class
 * implements 3, 4, 5 and 6.
 *
 * The precedence rule matters more than it looks: on an allow-listed
 * code-resolved route, the code's trainer outranks the session's selected
 * context. Without it, an existing player accepting a second trainer's
 * ShareLink resolves to the trainer already in their session, and the
 * membership insert is rejected by the RLS WITH CHECK — AC-01-13/14's own
 * edge case.
 *
 * @see specs/architect-architecture.md "Layer 3 — the mandatory tenant context"
 * @see specs/council-sharelink-tenant-resolution.md
 */
final class TenantResolver
{
    public const SESSION_KEY = '_active_trainer_id';

    public function __construct(
        private readonly AccountTrainerLinkRepository $links,
        private readonly PublicTenantCodeRepository $publicCodes,
    ) {
    }

    /**
     * @return int|null the resolved trainer id, or null when no source applies
     */
    public function resolve(Request $request, ?Account $account): ?int
    {
        // Source 5 — a public code carried in the route, on allow-listed
        // routes only. Outranks the session context deliberately.
        $codeTrainerId = $this->resolveFromPublicCode($request);

        if (null !== $codeTrainerId) {
            return $codeTrainerId;
        }

        if (null === $account) {
            return null;
        }

        // Source 4 — the trainer's own tenant.
        if (AccountRole::Trainer === $account->getRole()) {
            $owned = $this->links->findActiveTrainerIdsFor($account);

            if (1 === \count($owned)) {
                return $owned[0];
            }
        }

        // Source 3 — the selected trainer context, always validated against
        // the global AccountTrainerLink. An unvalidated candidate resolves
        // nothing; this validation is the security-bearing half.
        $candidate = $this->selectedCandidate($request);

        if (null !== $candidate && $this->links->isActiveLink($account, $candidate)) {
            return $candidate;
        }

        // A single-tenant player or coach needs no explicit selection.
        $available = $this->links->findActiveTrainerIdsFor($account);

        if (1 === \count($available)) {
            return $available[0];
        }

        // Source 6 — no tenant. Trainer-scoped access will throw.
        return null;
    }

    private function resolveFromPublicCode(Request $request): ?int
    {
        if (!TenantFromPublicCode::isAllowListed($request->attributes->get('_route'))) {
            return null;
        }

        $code = $request->attributes->get('code');

        if (!\is_string($code) || '' === $code) {
            return null;
        }

        return $this->publicCodes->findTrainerIdByCode($code);
    }

    private function selectedCandidate(Request $request): ?int
    {
        if (!$request->hasSession()) {
            return null;
        }

        $selected = $request->getSession()->get(self::SESSION_KEY);

        return \is_int($selected) ? $selected : null;
    }
}
