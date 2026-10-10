<?php

declare(strict_types=1);

namespace App\Scheduling\Service;

use App\Identity\Entity\PlayerProfile;
use App\Scheduling\Entity\CoachAssignment;
use App\Scheduling\Entity\Event;
use App\Scheduling\Entity\Rsvp;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;

/**
 * Every Epic-02 transactional email in one place, matching Identity's own
 * `IdentityMailer` — plain text `Email` objects rather than `TemplatedEmail`
 * + Twig, for the same reason: MVP scope favours the workflow and
 * authorization behind each send over template polish.
 *
 * "In-app" notifications named alongside "email" throughout the epic
 * (AC-02-11, AC-02-35, AC-02-40, AC-02-47) are the relevant list screen
 * itself — "Events to Confirm", "My Reservations", "My Activities" — the
 * same pattern `ChildApprovalService`'s own docblock establishes ("The
 * in-app half is the pending-approvals inbox itself... this is the email
 * half"), not a separate notification log entity (none exists in the
 * settled schema).
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md AC-02-11, AC-02-24, AC-02-26, AC-02-29, AC-02-32, AC-02-35, AC-02-36, AC-02-47, AC-02-51
 */
final readonly class SchedulingMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
        private PlayerAccountResolver $playerAccounts,
    ) {
    }

    /**
     * AC-02-24/26: "You're registered!" — sent once an RSVP reaches
     * Confirmed, free or paid alike.
     */
    public function sendRsvpConfirmed(Rsvp $rsvp): void
    {
        $event = $rsvp->getEvent();
        $this->sendToPlayer(
            $rsvp->getPlayer(),
            sprintf('You\'re registered for %s', $event->getTitle()),
            sprintf(
                "You're registered!\n\n%s\n%s at %s\n\nSee you there.",
                $event->getTitle(),
                $event->getStartsAt()->format('l, F j, Y \a\t g:i A'),
                $event->getLocation(),
            ),
        );
    }

    /**
     * AC-02-29: cancellation confirmation.
     */
    public function sendRsvpCanceled(Rsvp $rsvp): void
    {
        $event = $rsvp->getEvent();
        $this->sendToPlayer(
            $rsvp->getPlayer(),
            sprintf('Your RSVP for %s was canceled', $event->getTitle()),
            sprintf('Your registration for %s on %s has been canceled. Your spot has been released.', $event->getTitle(), $event->getStartsAt()->format('F j, Y')),
        );
    }

    /**
     * AC-02-32: sent only once the payment-intent gateway actually reports a
     * refund succeeded — never sent for a no-op/pending outcome, so this
     * email is never a false promise (see NoopPaymentIntentGateway's own
     * docblock).
     */
    public function sendRefundConfirmation(Rsvp $rsvp): void
    {
        $event = $rsvp->getEvent();
        $this->sendToPlayer(
            $rsvp->getPlayer(),
            sprintf('Refund processed for %s', $event->getTitle()),
            sprintf('Your refund for %s has been processed.', $event->getTitle()),
        );
    }

    /**
     * AC-02-11: the newly assigned coach.
     */
    public function sendCoachAssigned(CoachAssignment $assignment): void
    {
        $event = $assignment->getEvent();
        $this->send(
            $assignment->getCoachMembership()->getAccount()->getEmail(),
            sprintf('New assignment: %s', $event->getTitle()),
            sprintf(
                "You've been assigned to %s on %s at %s.\n\nConfirm or decline from your Activities page.",
                $event->getTitle(),
                $event->getStartsAt()->format('l, F j, Y \a\t g:i A'),
                $event->getLocation(),
            ),
        );
    }

    /**
     * AC-02-35: the trainer, once the coach confirms.
     */
    public function sendCoachAssignmentConfirmedToTrainer(CoachAssignment $assignment): void
    {
        $event = $assignment->getEvent();
        $this->send(
            $event->getTrainer()->getOwnerAccount()->getEmail(),
            sprintf('%s confirmed %s', $this->coachName($assignment), $event->getTitle()),
            sprintf('%s confirmed the assignment for %s.', $this->coachName($assignment), $event->getTitle()),
        );
    }

    /**
     * AC-02-36: the trainer, with the epic's own exact framing —
     * "[Coach] declined [Event]. Find replacement."
     */
    public function sendCoachAssignmentDeclinedToTrainer(CoachAssignment $assignment): void
    {
        $event = $assignment->getEvent();
        $this->send(
            $event->getTrainer()->getOwnerAccount()->getEmail(),
            sprintf('%s declined %s', $this->coachName($assignment), $event->getTitle()),
            sprintf('%s declined %s. Find replacement.', $this->coachName($assignment), $event->getTitle()),
        );
    }

    /**
     * AC-02-47: every registered player, once the trainer cancels the event.
     */
    public function sendEventCanceledToPlayer(Rsvp $rsvp): void
    {
        $event = $rsvp->getEvent();
        $this->sendToPlayer(
            $rsvp->getPlayer(),
            sprintf('%s has been canceled', $event->getTitle()),
            sprintf(
                "%s on %s has been canceled.\n\nReason: %s\n\nAny payment will be fully refunded.",
                $event->getTitle(),
                $event->getStartsAt()->format('F j, Y'),
                (string) $event->getCanceledReason(),
            ),
        );
    }

    /**
     * AC-02-47: "the assigned coach is notified that the assignment is
     * canceled."
     */
    public function sendEventCanceledToCoach(CoachAssignment $assignment): void
    {
        $event = $assignment->getEvent();
        $this->send(
            $assignment->getCoachMembership()->getAccount()->getEmail(),
            sprintf('%s has been canceled', $event->getTitle()),
            sprintf('%s on %s has been canceled. Your assignment is no longer needed.', $event->getTitle(), $event->getStartsAt()->format('F j, Y')),
        );
    }

    /**
     * AC-02-51: date/time or location change notifies RSVP'd players.
     */
    public function sendEventDetailsChanged(Rsvp $rsvp, Event $event): void
    {
        $this->sendToPlayer(
            $rsvp->getPlayer(),
            sprintf('%s has been updated', $event->getTitle()),
            sprintf(
                "The details for %s have changed.\n\nNew date/time: %s\nNew location: %s",
                $event->getTitle(),
                $event->getStartsAt()->format('l, F j, Y \a\t g:i A'),
                $event->getLocation(),
            ),
        );
    }

    /**
     * AC-02-51: "if the price increased, allows them to cancel with a
     * refund" — the email states that option explicitly.
     */
    public function sendPriceChanged(Rsvp $rsvp, Event $event, bool $increased): void
    {
        $body = $increased
            ? sprintf('The price for %s has increased. You may cancel your RSVP for a full refund if you no longer wish to attend.', $event->getTitle())
            : sprintf('The price for %s has changed.', $event->getTitle());

        $this->sendToPlayer($rsvp->getPlayer(), sprintf('Price update for %s', $event->getTitle()), $body);
    }

    /**
     * AC-02-51: "a coach change notifies both the old and new coach."
     */
    public function sendCoachReassignmentNotice(CoachAssignment $assignment, bool $isNewCoach): void
    {
        $event = $assignment->getEvent();
        $subject = $isNewCoach ? sprintf('You are now assigned to %s', $event->getTitle()) : sprintf('You are no longer assigned to %s', $event->getTitle());
        $body = $isNewCoach
            ? sprintf('You have been assigned to %s on %s.', $event->getTitle(), $event->getStartsAt()->format('F j, Y'))
            : sprintf('A different coach has been assigned to %s. You are no longer assigned to this event.', $event->getTitle());

        $this->send($assignment->getCoachMembership()->getAccount()->getEmail(), $subject, $body);
    }

    /**
     * Resolves the responsible contact for a player: their own account, if
     * they have one, else the parent on record (a child usually has no
     * login of their own — matching how ChildApprovalRequest routes every
     * notification to the parent account it carries directly).
     */
    private function sendToPlayer(PlayerProfile $player, string $subject, string $text): void
    {
        $account = $this->playerAccounts->resolve($player);

        if (null === $account) {
            // No reachable contact on record. Nothing to notify — the in-app
            // surface (My Reservations) still reflects the change.
            return;
        }

        $this->send($account->getEmail(), $subject, $text);
    }

    private function coachName(CoachAssignment $assignment): string
    {
        $account = $assignment->getCoachMembership()->getAccount();
        $profile = $account->getProfile();

        return $profile?->getFullName() ?? $account->getEmail();
    }

    private function send(string $to, string $subject, string $text): void
    {
        $email = (new Email())
            ->from(self::FROM)
            ->to($to)
            ->subject($subject)
            ->text($text);

        $this->mailer->send($email);
    }
}
