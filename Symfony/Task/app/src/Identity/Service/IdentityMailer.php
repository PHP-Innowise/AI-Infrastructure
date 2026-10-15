<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\ChildApprovalRequest;
use App\Platform\Entity\Trainer;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * Every Epic-01 transactional email in one place. Plain text/HTML `Email`
 * objects rather than `TemplatedEmail` + Twig templates: MVP scope favours
 * the workflow and authorization behind each send over template polish —
 * see the coder's final report for this recorded as a deliberate
 * simplification, not an oversight.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md AC-01-3, AC-01-4, AC-01-12, AC-01-25, AC-01-26, AC-01-31, AC-01-39, AC-01-42, BR-01-4, BR-01-5
 */
final readonly class IdentityMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
        private UrlGeneratorInterface $urlGenerator,
    ) {
    }

    /**
     * AC-01-3, AC-01-4: the new trainer's setup link.
     */
    public function sendTrainerSetupInvite(Account $trainer, string $rawToken): void
    {
        $url = $this->urlGenerator->generate('identity_account_setup', ['token' => $rawToken], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $trainer->getEmail(),
            'Set up your PracticePerfect trainer account',
            sprintf("Welcome to PracticePerfect!\n\nSet your password to get started: %s\n\nThis link expires in 1 hour.", $url),
        );
    }

    /**
     * BR-01-4: password reset.
     */
    public function sendPasswordReset(Account $account, string $rawToken): void
    {
        $url = $this->urlGenerator->generate('identity_password_reset', ['token' => $rawToken], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $account->getEmail(),
            'Reset your PracticePerfect password',
            sprintf("Reset your password: %s\n\nThis link expires in 1 hour. If you did not request this, ignore this email.", $url),
        );
    }

    /**
     * BR-01-5: email verification.
     */
    public function sendEmailVerification(Account $account, string $rawToken): void
    {
        $url = $this->urlGenerator->generate('identity_email_verify', ['token' => $rawToken], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $account->getEmail(),
            'Verify your PracticePerfect email address',
            sprintf("Confirm your email address: %s\n\nThis link expires in 24 hours.", $url),
        );
    }

    /**
     * AC-01-12: confirmation after a player/parent registers via ShareLink.
     */
    public function sendRegistrationConfirmation(Account $account, Trainer $trainer): void
    {
        $this->send(
            $account->getEmail(),
            sprintf('Welcome to %s on PracticePerfect', $trainer->getBusinessName()),
            sprintf("You're all set. You are now connected with %s on PracticePerfect.", $trainer->getBusinessName()),
        );
    }

    /**
     * AC-01-39, BR-01-15: the coach invitation itself.
     */
    public function sendCoachInvite(string $targetEmail, Trainer $trainer, string $code): void
    {
        $url = $this->urlGenerator->generate('identity_invite_show', ['code' => $code], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $targetEmail,
            sprintf('%s invited you to coach on PracticePerfect', $trainer->getBusinessName()),
            sprintf("You've been invited to coach for %s.\n\nAccept your invitation: %s\n\nThis link expires in 7 days.", $trainer->getBusinessName(), $url),
        );
    }

    /**
     * AC-01-25: the parent is notified by email that a child's request is
     * pending their approval.
     */
    public function sendApprovalRequested(ChildApprovalRequest $request): void
    {
        $this->send(
            $request->getParentAccount()->getEmail(),
            'Approval needed for your child on PracticePerfect',
            sprintf(
                "%s has requested a %s that needs your approval.\n\nReview it in your Approvals inbox. This request expires in 48 hours if you do not respond.",
                $request->getChildPlayer()->getFirstName(),
                $request->getActionType(),
            ),
        );
    }

    /**
     * AC-01-26: the child is notified once the parent decides.
     */
    public function sendApprovalDecided(ChildApprovalRequest $request): void
    {
        $childAccountEmail = $request->getChildPlayer()->getSelfAccount()?->getEmail();

        if (null === $childAccountEmail) {
            // No separate child login (the common case) — nothing to notify.
            return;
        }

        $this->send(
            $childAccountEmail,
            'Your PracticePerfect request has been decided',
            sprintf('Your parent has %s your request.', $request->getStatus()),
        );
    }

    /**
     * AC-01-31: a logged-in child clicked a new trainer's ShareLink. The
     * child is blocked; the parent gets a "Review Registration" CTA instead.
     */
    public function sendParentReviewRegistration(Account $parent, Account $child, Trainer $trainer, string $code): void
    {
        $url = $this->urlGenerator->generate('identity_sharelink_show', ['code' => $code], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $parent->getEmail(),
            sprintf('%s wants to train with %s', $child->getEmail(), $trainer->getBusinessName()),
            sprintf("Review and complete this registration: %s", $url),
        );
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
