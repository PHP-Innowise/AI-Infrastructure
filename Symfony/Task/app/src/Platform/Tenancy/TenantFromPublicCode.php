<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

/**
 * The allow-list of routes whose tenant is resolved from a public code.
 *
 * This list is the whole security boundary for code-resolved routes: a route
 * added here can establish a tenant from a bearer code carried by an anonymous
 * request. Keeping it in one enumerable place is what makes growing it a
 * reviewed act rather than an accident.
 *
 * Resolution is not authorization. Establishing the tenant only makes the
 * trainer's tables reachable; whether the actor may do anything is still a
 * voter decision on the trainer-scoped row the code points at.
 *
 * @see specs/council-sharelink-tenant-resolution.md "Recommendation"
 */
final class TenantFromPublicCode
{
    /**
     * @var list<string>
     */
    private const ROUTES = [
        // Epic-01 — ShareLink acceptance (player static links).
        'identity_sharelink_show',
        'identity_sharelink_register',
        'identity_sharelink_associate',
        // Epic-01 — ShareLink acceptance (unique coach invite links, and
        // coach-issued player invites, which reuse the same shape).
        'identity_invite_show',
        'identity_invite_register',
        // Epic-08 — public camp and evaluation forms.
        'forms_public_show',
        'forms_public_submit',
        'forms_public_confirmation',
        // AC-08-23..26: account conversion stays inside the code-resolved
        // flow — same trainer the form belongs to, still no session context
        // (api-designer-spec.md: "the account and the membership are
        // created in the same request that is still running under source
        // 5's tenant resolution").
        'forms_public_convert_account',
        // Stripe Checkout return waypoints — also code-resolved, so the
        // submission lookup they perform runs under an established tenant
        // rather than failing closed on a fresh, session-less redirect back
        // from Stripe.
        'forms_public_checkout_success',
        'forms_public_checkout_cancel',
    ];

    public static function isAllowListed(mixed $routeName): bool
    {
        return \is_string($routeName) && \in_array($routeName, self::ROUTES, true);
    }

    /**
     * @return list<string>
     */
    public static function routes(): array
    {
        return self::ROUTES;
    }
}
