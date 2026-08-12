<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAssignment;
use App\Content\Repository\ContentProgressRepository;
use App\Content\Repository\PlaylistAssignmentRepository;
use App\Content\Repository\PlaylistItemRepository;
use App\Crm\Entity\Label;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerProfileRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-04.05: assigning a playlist to players, a label group, or a
 * skill-level filter, and reporting assignment status.
 *
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-13..15, AC-04-13..16
 */
final readonly class PlaylistAssignmentService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlaylistAssignmentRepository $assignments,
        private ContentAssignmentResolver $resolver,
        private ContentProgressRepository $progress,
        private PlaylistItemRepository $playlistItems,
        private PlayerProfileRepository $playerProfiles,
        private ContentMailer $mailer,
    ) {
    }

    /**
     * AC-04-13/14, BR-04-13/14: creates ONE assignment act — a group target
     * covers an unbounded, membership-dependent set of players resolved at
     * read time (`ContentAssignmentResolver`), never fanned out into one row
     * per player here. Notifies (email) every player CURRENTLY covered, at
     * the moment of assignment — a player who joins the target group later
     * gets no retroactive email, only the (always current) in-app listing.
     * BR-04-15: reassigning the same playlist to a player who already has
     * it is simply a second assignment row — existing `ContentProgress` is
     * untouched either way, since progress was never linked to the
     * assignment row in the first place.
     */
    public function assign(
        Trainer $trainer,
        Playlist $playlist,
        Account $actor,
        string $targetType,
        ?PlayerProfile $targetPlayer,
        ?Label $targetLabel,
        ?string $targetSkillLevel,
        ?\DateTimeImmutable $dueDate,
        ?string $note,
    ): PlaylistAssignment {
        $assignment = $this->entityManager->wrapInTransaction(function () use (
            $trainer, $playlist, $actor, $targetType, $targetPlayer, $targetLabel, $targetSkillLevel, $dueDate, $note,
        ): PlaylistAssignment {
            $assignment = new PlaylistAssignment($trainer, $playlist, $targetType, $actor, $targetPlayer, $targetLabel, $targetSkillLevel, $dueDate, $note);
            $this->assignments->add($assignment);
            $this->entityManager->flush();

            return $assignment;
        });

        foreach ($this->resolver->coveredPlayerIds($trainer, $playlist) as $playerId) {
            $player = $this->playerProfiles->find($playerId);

            if (null !== $player) {
                $this->mailer->sendPlaylistAssigned($player, $playlist);
            }
        }

        return $assignment;
    }

    /**
     * AC-04-15: "Assigned to 15 players, 8 completed, 7 in progress." A
     * covered player counts as completed only when EVERY required item in
     * the playlist is complete for them; in-progress when they have any
     * progress row at all short of that; not-started otherwise (counted in
     * $assigned but neither of the other two buckets).
     *
     * @return array{assigned: int, completed: int, inProgress: int}
     */
    public function statusCounts(Trainer $trainer, Playlist $playlist): array
    {
        $requiredItemIds = array_map(
            static fn ($item): int => (int) $item->getContentItem()->getId(),
            array_values(array_filter($this->playlistItems->findForPlaylist($playlist), static fn ($item): bool => $item->isRequired())),
        );

        $completed = 0;
        $inProgress = 0;
        $coveredPlayerIds = $this->resolver->coveredPlayerIds($trainer, $playlist);

        foreach ($coveredPlayerIds as $playerId) {
            $player = $this->playerProfiles->find($playerId);

            if (null === $player) {
                continue;
            }

            $progressRows = $this->progress->findForPlayer($trainer, $player);
            $completedItemIds = [];

            foreach ($progressRows as $row) {
                if ($row->isCompleted()) {
                    $completedItemIds[(int) $row->getContentItem()->getId()] = true;
                }
            }

            $completedRequired = [] === $requiredItemIds
                ? false
                : [] === array_diff($requiredItemIds, array_keys($completedItemIds));

            if ($completedRequired) {
                ++$completed;
            } elseif ([] !== $completedItemIds) {
                ++$inProgress;
            }
        }

        return ['assigned' => \count($coveredPlayerIds), 'completed' => $completed, 'inProgress' => $inProgress];
    }
}
