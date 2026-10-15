<?php

declare(strict_types=1);

namespace App\Scheduling\Voter;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Identity\Service\PlayerContextResolver;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use App\Scheduling\Repository\RsvpRepository;
use App\Scheduling\Service\EventEligibilityChecker;
use Symfony\Component\HttpFoundation\RequestStack;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-02-7..11, AC-02-23..33.
 *
 * `RSVP_CREATE`'s capacity check is deliberately the UNLOCKED,
 * non-authoritative pre-check architect-architecture.md's "Lock ordering"
 * names explicitly ("An unlocked capacity pre-check gives fail-fast UX for
 * the common 'event full' rejection... the locked check inside the
 * transaction is the one that decides") — RsvpService re-checks under
 * `SELECT ... FOR UPDATE` regardless of what this voter concludes.
 *
 * `RSVP_REMOVE` is new (specs/api-designer-spec.md:479): the trainer's own
 * roster-management removal, distinct from the player's own
 * `RSVP_CANCEL` — different actor, different subject-ownership rule, same
 * `Rsvp` subject type, so one voter class covers both without either
 * `supports()` clause straddling unrelated logic.
 *
 * @see specs/security-voter-designer-design.md "Scheduling module"
 */
/**
 * @extends Voter<string, Event|Rsvp>
 */
final class RsvpVoter extends Voter
{
    public const RSVP_CREATE = 'RSVP_CREATE';
    public const RSVP_CANCEL = 'RSVP_CANCEL';
    public const RSVP_REMOVE = 'RSVP_REMOVE';

    public function __construct(
        private readonly RequestStack $requestStack,
        private readonly PlayerContextResolver $playerContext,
        private readonly PlayerTrainerMembershipRepository $memberships,
        private readonly ParentChildLinkRepository $parentChildLinks,
        private readonly EventEligibilityChecker $eligibility,
        private readonly RsvpRepository $rsvps,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::RSVP_CREATE => $subject instanceof Event,
            self::RSVP_CANCEL, self::RSVP_REMOVE => $subject instanceof Rsvp,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        return match (true) {
            self::RSVP_CREATE === $attribute && $subject instanceof Event => $this->voteCreate($subject, $actor),
            self::RSVP_CANCEL === $attribute && $subject instanceof Rsvp => $this->voteCancel($subject, $actor),
            self::RSVP_REMOVE === $attribute && $subject instanceof Rsvp => $this->voteRemove($actor),
            default => false,
        };
    }

    /**
     * BR-02-7: once per event, eligible, not past, not full (fail-fast
     * only — see the class docblock).
     */
    private function voteCreate(Event $event, Account $actor): bool
    {
        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        $request = $this->requestStack->getCurrentRequest();

        if (null === $request) {
            return false;
        }

        $player = $this->playerContext->resolve($request, $actor);

        if (Event::STATUS_ACTIVE !== $event->getStatus() || $event->hasStarted(new \DateTimeImmutable())) {
            return false;
        }

        if (null !== $this->rsvps->findActiveOneByEventAndPlayer($event, $player)) {
            return false;
        }

        $membership = $this->memberships->findOneByTrainerAndPlayer($event->getTrainer(), $player);

        if (null === $membership || !$membership->isActive() || !$this->eligibility->isEligible($event, $membership)) {
            return false;
        }

        return $this->rsvps->countHeld($event) < $event->getCapacity();
    }

    /**
     * AC-02-29/30: the player (or their parent) may cancel their own RSVP,
     * only before the event starts.
     */
    private function voteCancel(Rsvp $rsvp, Account $actor): bool
    {
        if (AccountRole::Player !== $actor->getRole()) {
            return false;
        }

        if (!$this->ownsPlayer($rsvp, $actor)) {
            return false;
        }

        return !$rsvp->getEvent()->hasStarted(new \DateTimeImmutable());
    }

    /**
     * US-02.12: trainer-only roster removal — structurally same-tenant
     * already (RLS + the Doctrine filter).
     */
    private function voteRemove(Account $actor): bool
    {
        return AccountRole::Trainer === $actor->getRole();
    }

    private function ownsPlayer(Rsvp $rsvp, Account $actor): bool
    {
        $player = $rsvp->getPlayer();

        if ($player->getSelfAccount() === $actor) {
            return true;
        }

        $link = $this->parentChildLinks->findByChildPlayer($player);

        return null !== $link && $link->getParentAccount() === $actor;
    }
}
