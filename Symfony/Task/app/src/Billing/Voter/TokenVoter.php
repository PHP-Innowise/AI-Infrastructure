<?php

declare(strict_types=1);

namespace App\Billing\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Tenancy\TenantContext;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * AC-05-4..6, AC-05-29, AC-05-33: token purchases, subscription purchases,
 * and trainer gifting.
 *
 * `TOKEN_GIFT`'s own subject is `PlayerTrainerMembership`, matching
 * specs/api-designer-spec.md's route table exactly
 * (`billing_trainer_gift_tokens`) — gifting targets a specific player
 * within the trainer's own roster, not a bare capability check.
 *
 * `TOKEN_PURCHASE`/`TOKEN_SUBSCRIPTION_PURCHASE` need no object: "the query
 * itself is scoped to the current actor" (api-designer-spec.md "Gates vs.
 * voters" — a player buying tokens always buys for their OWN current
 * trainer context, resolved server-side, never a client-supplied trainer
 * id).
 *
 * @see specs/api-designer-spec.md "Billing module"
 */
/**
 * @extends Voter<string, PlayerTrainerMembership|null>
 */
final class TokenVoter extends Voter
{
    public const TOKEN_GIFT = 'TOKEN_GIFT';
    public const TOKEN_PURCHASE = 'TOKEN_PURCHASE';
    public const TOKEN_SUBSCRIPTION_PURCHASE = 'TOKEN_SUBSCRIPTION_PURCHASE';

    public function __construct(
        private readonly TenantContext $tenantContext,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::TOKEN_GIFT => $subject instanceof PlayerTrainerMembership,
            self::TOKEN_PURCHASE, self::TOKEN_SUBSCRIPTION_PURCHASE => null === $subject,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        if (self::TOKEN_GIFT === $attribute) {
            \assert($subject instanceof PlayerTrainerMembership);

            // AC-05-33: trainers only, own tenant only — structurally
            // same-tenant already (RLS + the Doctrine filter make a
            // foreign-tenant membership invisible before this voter runs),
            // matching RsvpVoter::voteRemove()'s own precedent.
            return AccountRole::Trainer === $actor->getRole()
                && $subject->getTrainer()->getId() === $this->tenantContext->getTrainerIdOrNull();
        }

        // AC-05-4/29: any player/parent may attempt a purchase for their
        // own current trainer context — a child login's OWN attempt is not
        // denied here (it triggers parent approval instead, exactly like
        // RsvpVoter::RSVP_CREATE / PlaylistVoter::PLAYLIST_PURCHASE), only
        // a non-player role is.
        return AccountRole::Player === $actor->getRole();
    }
}
