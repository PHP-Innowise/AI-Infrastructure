<?php

declare(strict_types=1);

namespace App\Identity\Service;

/**
 * How a child's RSVP or purchase attempt would be paid for — the axis
 * `ChildApprovalVoter::CHILD_APPROVAL_BYPASS` branches on, per
 * specs/security-voter-designer-design.md "Parent-child approval".
 */
enum FundingMethod: string
{
    case Free = 'free';
    case Usd = 'usd';
    case Token = 'token';
}
