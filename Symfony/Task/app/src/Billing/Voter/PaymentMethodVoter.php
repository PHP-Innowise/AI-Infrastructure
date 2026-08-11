<?php

declare(strict_types=1);

namespace App\Billing\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\ParentChildLinkRepository;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * AC-05-20/21: managing saved payment methods — "denies a child login
 * outright" (AC-01-30's own rule, applied here the same way
 * `App\Identity\Voter\ChildProfileVoter` applies it: the distinguishing
 * test is which `Account` `getUser()` returns, not which `PlayerProfile` is
 * targeted).
 *
 * @see specs/api-designer-spec.md "Billing module"
 */
/**
 * @extends Voter<string, null>
 */
final class PaymentMethodVoter extends Voter
{
    public const PAYMENT_METHOD_MANAGE = 'PAYMENT_METHOD_MANAGE';

    public function __construct(
        private readonly ParentChildLinkRepository $parentChildLinks,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return self::PAYMENT_METHOD_MANAGE === $attribute && null === $subject;
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account || AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        // AC-01-30: a child's own login is denied outright — payment
        // methods are a whole-family, parent-only concern (AC-05-21).
        return null === $this->parentChildLinks->findByChildAccount($actor);
    }
}
