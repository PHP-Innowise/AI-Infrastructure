<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\Playlist;
use App\Identity\Entity\PlayerProfile;
use Symfony\Component\Mailer\MailerInterface;
use Symfony\Component\Mime\Email;

/**
 * Every Epic-04 transactional email, matching `App\Scheduling\Service\SchedulingMailer`'s
 * own precedent — plain-text `Email`, not `TemplatedEmail` + Twig.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md AC-04-14
 */
final readonly class ContentMailer
{
    private const FROM = 'no-reply@practiceperfect.test';

    public function __construct(
        private MailerInterface $mailer,
        private PlayerAccountResolver $playerAccounts,
    ) {
    }

    /**
     * AC-04-14: "New content assigned: [Playlist Name]" with a link to the
     * LPPP portal. The "link to the LPPP portal" is the fixed `/portal/content`
     * path — there is no per-notification deep link entity in this schema
     * (matching how SchedulingMailer's own emails link nowhere beyond plain
     * text; "in-app" notifications throughout this codebase ARE the
     * relevant list screen itself, not a separate notification log).
     */
    public function sendPlaylistAssigned(PlayerProfile $player, Playlist $playlist): void
    {
        $account = $this->playerAccounts->resolve($player);

        if (null === $account) {
            return;
        }

        $email = (new Email())
            ->from(self::FROM)
            ->to($account->getEmail())
            ->subject(sprintf('New content assigned: %s', $playlist->getTitle()))
            ->text(sprintf(
                "New content has been assigned to you: %s\n\nVisit your LPPP portal to view it: /portal/content",
                $playlist->getTitle(),
            ));

        $this->mailer->send($email);
    }
}
