<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\PlaylistAccessGrant;

/**
 * The result of one `PurchasePlaylistAccessService::purchase()`/
 * `completeAfterParentApproval()` attempt — exactly one of four shapes,
 * exposed as named constructors so a caller (`PortalContentController`)
 * cannot construct an invalid combination:
 *
 * - `granted`: access unlocked now (token spend succeeded, or the playlist
 *   was already owned).
 * - `redirect`: a card payment needs a Stripe Checkout redirect, in this
 *   same request/response cycle — specs/api-designer-spec.md "Billing
 *   module": "no separate 'create checkout session' endpoint."
 * - `pendingApproval`: a child's attempt is queued for the parent
 *   (`ChildApprovalRequest`).
 * - `failed`: the payment gateway reported failure (e.g. an
 *   insufficient-balance race after the caller's own pre-check).
 */
final readonly class PlaylistPurchaseOutcome
{
    private function __construct(
        public ?PlaylistAccessGrant $grant,
        public ?string $redirectUrl,
        public bool $pendingApproval,
        public bool $failed,
    ) {
    }

    public static function granted(PlaylistAccessGrant $grant): self
    {
        return new self($grant, null, false, false);
    }

    public static function redirect(string $redirectUrl): self
    {
        return new self(null, $redirectUrl, false, false);
    }

    public static function pendingApproval(): self
    {
        return new self(null, null, true, false);
    }

    public static function failed(): self
    {
        return new self(null, null, false, true);
    }
}
