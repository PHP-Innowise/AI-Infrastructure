<?php

declare(strict_types=1);

namespace App\Content\Service;

use App\Content\Entity\Playlist;
use App\Content\Entity\PlaylistAssignment;
use App\Content\Repository\PlaylistAssignmentRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerTrainerMembershipRepository;
use App\Platform\Entity\Trainer;

/**
 * BR-04-13/14/15: resolves WHICH PLAYERS a `PlaylistAssignment` row
 * currently covers, against CURRENT label/skill-level membership — never a
 * fan-out frozen at assignment time (see that entity's own docblock).
 *
 * Reads Crm's `PlayerLabelRepository` directly, per
 * architect-architecture.md "Module map": Content "may call... Crm (read
 * labels for group assignment)" — through Crm's own repository, never by
 * Content reimplementing label logic.
 *
 * @see specs/database-designer-schema.md "`playlist_assignment`" — "A
 *      player's assignment-level status is derived at read time..."
 */
final readonly class ContentAssignmentResolver
{
    public function __construct(
        private PlaylistAssignmentRepository $assignments,
        private PlayerLabelRepository $playerLabels,
        private PlayerTrainerMembershipRepository $memberships,
    ) {
    }

    /**
     * AC-04-23: is this playlist one of the CURRENT player's assigned
     * Practice workouts (or, on the Learn tab, "Suggested" content)?
     */
    public function isAssignedToPlayer(Trainer $trainer, Playlist $playlist, PlayerProfile $player): bool
    {
        foreach ($this->assignments->findForPlaylist($playlist) as $assignment) {
            if ($this->matches($assignment, $trainer, $player)) {
                return true;
            }
        }

        return false;
    }

    /**
     * AC-04-23: every DISTINCT playlist of the given pillar covered by ANY
     * of this player's current assignments (direct, by label, or by skill
     * level) — the Practice tab's own list, and the Learn tab's "Suggested"
     * badge source.
     *
     * @return list<Playlist>
     */
    public function assignedPlaylistsForPlayer(Trainer $trainer, PlayerProfile $player, string $pillar): array
    {
        $matched = [];

        foreach ($this->assignments->findAllForActiveTenant($trainer) as $assignment) {
            $playlist = $assignment->getPlaylist();

            if ($playlist->getPillar() !== $pillar || $playlist->isDeleted()) {
                continue;
            }

            if (isset($matched[(int) $playlist->getId()])) {
                continue;
            }

            if ($this->matches($assignment, $trainer, $player)) {
                $matched[(int) $playlist->getId()] = $playlist;
            }
        }

        return array_values($matched);
    }

    /**
     * AC-04-28: "sorted by nearest due date first" — the EARLIEST due date
     * among this playlist's own assignment rows that currently match the
     * player (a playlist may carry several assignment rows, e.g. one
     * direct + one by label, each with its own optional due date). `null`
     * when no matching row carries one.
     */
    public function nearestDueDateForPlayer(Trainer $trainer, Playlist $playlist, PlayerProfile $player): ?\DateTimeImmutable
    {
        $nearest = null;

        foreach ($this->assignments->findForPlaylist($playlist) as $assignment) {
            $dueDate = $assignment->getDueDate();

            if (null === $dueDate || !$this->matches($assignment, $trainer, $player)) {
                continue;
            }

            if (null === $nearest || $dueDate < $nearest) {
                $nearest = $dueDate;
            }
        }

        return $nearest;
    }

    /**
     * AC-04-15: "Assigned to 15 players" — every player CURRENTLY covered by
     * ANY assignment row for this playlist, deduplicated.
     *
     * @return list<int>
     */
    public function coveredPlayerIds(Trainer $trainer, Playlist $playlist): array
    {
        $covered = [];

        foreach ($this->assignments->findForPlaylist($playlist) as $assignment) {
            foreach ($this->resolveTargetPlayerIds($assignment, $trainer) as $playerId) {
                $covered[$playerId] = true;
            }
        }

        return array_keys($covered);
    }

    private function matches(PlaylistAssignment $assignment, Trainer $trainer, PlayerProfile $player): bool
    {
        return match ($assignment->getTargetType()) {
            PlaylistAssignment::TARGET_PLAYER => $assignment->getTargetPlayer()?->getId() === $player->getId(),
            PlaylistAssignment::TARGET_LABEL => $this->playerHasLabel($assignment, $player),
            PlaylistAssignment::TARGET_SKILL_LEVEL => $this->playerHasSkillLevel($trainer, $assignment, $player),
            default => false,
        };
    }

    private function playerHasLabel(PlaylistAssignment $assignment, PlayerProfile $player): bool
    {
        $label = $assignment->getTargetLabel();

        if (null === $label) {
            return false;
        }

        foreach ($this->playerLabels->findForPlayer($player) as $playerLabel) {
            if ($playerLabel->getLabel()->getId() === $label->getId()) {
                return true;
            }
        }

        return false;
    }

    private function playerHasSkillLevel(Trainer $trainer, PlaylistAssignment $assignment, PlayerProfile $player): bool
    {
        $membership = $this->memberships->findOneByTrainerAndPlayer($trainer, $player);

        return null !== $membership
            && $membership->isActive()
            && $membership->getSkillLevel() === $assignment->getTargetSkillLevel();
    }

    /**
     * @return list<int>
     */
    private function resolveTargetPlayerIds(PlaylistAssignment $assignment, Trainer $trainer): array
    {
        return match ($assignment->getTargetType()) {
            PlaylistAssignment::TARGET_PLAYER => null !== $assignment->getTargetPlayer() ? [(int) $assignment->getTargetPlayer()->getId()] : [],
            PlaylistAssignment::TARGET_LABEL => null !== $assignment->getTargetLabel() ? $this->playerLabels->findPlayerIdsForLabel($assignment->getTargetLabel()) : [],
            PlaylistAssignment::TARGET_SKILL_LEVEL => $this->activePlayerIdsForSkillLevel($trainer, $assignment->getTargetSkillLevel()),
            default => [],
        };
    }

    /**
     * @return list<int>
     */
    private function activePlayerIdsForSkillLevel(Trainer $trainer, ?string $skillLevel): array
    {
        if (null === $skillLevel) {
            return [];
        }

        $ids = [];

        foreach ($this->memberships->findActiveForActiveTenant() as $membership) {
            if ($membership->getSkillLevel() === $skillLevel && $membership->getTrainer()->getId() === $trainer->getId()) {
                $ids[] = (int) $membership->getPlayer()->getId();
            }
        }

        return $ids;
    }
}
