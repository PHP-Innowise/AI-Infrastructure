<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;

/**
 * The subject of `ChildApprovalVoter::CHILD_APPROVAL_BYPASS`. A typed DTO
 * rather than a bare PlayerProfile: the bypass rule depends on *how* the
 * action is funded (BR-01-18 vs BR-01-19), which a bare profile can't carry
 * without the caller pre-branching — exactly what the architecture forbids.
 *
 * @see specs/security-voter-designer-design.md "Parent-child approval" and its Decisions row
 */
final readonly class ChildActionAttempt
{
    public function __construct(
        public Account $actor,
        public PlayerProfile $beneficiary,
        public FundingMethod $fundingMethod,
    ) {
    }
}
