<?php

declare(strict_types=1);

namespace App\Forms\Service;

use App\Forms\Entity\FormSubmission;
use App\Identity\Repository\ShareLinkRepository;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;
use Symfony\Component\Routing\Generator\UrlGeneratorInterface;

/**
 * Every Epic-08 transactional email, matching `IdentityMailer`/
 * `BillingMailer`'s own precedent — plain-text `Email`, not `TemplatedEmail`
 * + Twig (recorded as a deliberate simplification, not an oversight, same
 * as those two).
 */
final readonly class FormsMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
        private UrlGeneratorInterface $urlGenerator,
        private SubmissionTokenFactory $tokens,
        private ShareLinkRepository $shareLinks,
    ) {
    }

    /**
     * AC-08-16/23/25: one email, sent once, carrying everything the epic
     * spreads across three ACs. "Declining conversion is not a route — it
     * is simply not clicking the CTA" (api-designer-spec.md), so there is
     * no later event to hang a separate "you never converted" email off —
     * this codebase has no scheduled/delayed-send infrastructure either
     * (no Symfony Scheduler job is wired anywhere). Folding AC-08-25's
     * "register later" link into the SAME immediate confirmation is the
     * only buildable reading that does not silently drop it for whoever
     * never clicks through: the "Create Your Account" link and the
     * "register later" link both reach every registrant, on the one email
     * that actually gets sent, regardless of which one they act on.
     */
    public function sendConfirmation(FormSubmission $submission): void
    {
        $form = $submission->getForm();
        $confirmationUrl = $this->urlGenerator->generate(
            'forms_public_confirmation',
            ['code' => $form->getShareableSlug(), 'submission' => $this->tokens->tokenFor($submission)],
            UrlGeneratorInterface::ABSOLUTE_URL,
        );

        $lines = [
            sprintf('Your registration for "%s" is confirmed.', $form->getName()),
        ];

        if (null !== $form->getDescription()) {
            $lines[] = $form->getDescription();
        }

        $lines[] = sprintf('Create a free account to access the training calendar and more: %s', $confirmationUrl);

        $laterUrl = $this->registerLaterUrl($form->getTrainer()->getId());

        if (null !== $laterUrl) {
            $lines[] = sprintf("Not ready yet? No problem — you can create an account anytime here: %s", $laterUrl);
        }

        $this->send($submission->getContactEmail(), sprintf('Registration confirmed: %s', $form->getName()), implode("\n\n", $lines));
    }

    /**
     * AC-08-26: the submitter's email already has an account — prompted to
     * log in instead of creating a duplicate.
     */
    public function sendLogInInsteadNotice(FormSubmission $submission): void
    {
        $url = $this->urlGenerator->generate('identity_auth_login', [], UrlGeneratorInterface::ABSOLUTE_URL);

        $this->send(
            $submission->getContactEmail(),
            'You already have a PracticePerfect account',
            sprintf('An account already exists for this email address. Log in to link your registration: %s', $url),
        );
    }

    /**
     * AC-08-25: the trainer's existing static player link (AC-01-73: one
     * per trainer, provisioned at trainer creation), not a fresh
     * submission-specific grant — "register later" describes the same
     * general player self-registration path any visitor uses.
     */
    private function registerLaterUrl(?int $trainerId): ?string
    {
        if (null === $trainerId) {
            return null;
        }

        $staticLink = $this->shareLinks->findStaticPlayerLink($trainerId);

        if (null === $staticLink) {
            // Every trainer is provisioned one at creation
            // (TrainerProvisioningService) — absence here means a fixture or
            // test built a Trainer directly, not a real gap.
            return null;
        }

        return $this->urlGenerator->generate('identity_sharelink_show', ['code' => $staticLink->getCode()], UrlGeneratorInterface::ABSOLUTE_URL);
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
