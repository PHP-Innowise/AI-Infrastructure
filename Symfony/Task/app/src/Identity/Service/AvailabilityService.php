<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\AvailabilityWindow;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\AvailabilityWindowRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * US-01.09/US-01.10: "Best Times" for players/parents and coaches.
 *
 * BR-01-25: availability drives scheduling suggestions, not hard
 * restrictions — nothing here ever blocks an RSVP or assignment; it is
 * read by whoever plans one (Epic-02, once it exists).
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.09, US-01.10, AC-01-43..46
 */
final readonly class AvailabilityService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AvailabilityWindowRepository $windows,
    ) {
    }

    /**
     * AC-01-43/44: replaces the player's (or, per child, the selected
     * child's) whole weekly grid in one save — simpler and less error-prone
     * for the caller than a diff against the previous set.
     *
     * @param list<array{dayOfWeek: int, startTime: \DateTimeImmutable, endTime: \DateTimeImmutable, isAvailable: bool}> $slots
     */
    public function setPlayerAvailability(Trainer $trainer, PlayerProfile $player, array $slots): void
    {
        $this->entityManager->wrapInTransaction(function () use ($trainer, $player, $slots): void {
            $this->windows->removeAllForPlayer($player);
            $this->entityManager->flush();

            foreach ($slots as $slot) {
                $this->windows->add(AvailabilityWindow::forPlayer(
                    $trainer,
                    $player,
                    $slot['dayOfWeek'],
                    $slot['startTime'],
                    $slot['endTime'],
                    $slot['isAvailable'],
                ));
            }
        });
    }

    /**
     * AC-01-46: a recurring weekly schedule, multiple slots per day allowed.
     *
     * @param list<array{dayOfWeek: int, startTime: \DateTimeImmutable, endTime: \DateTimeImmutable, isAvailable: bool}> $slots
     */
    public function setCoachAvailability(Trainer $trainer, CoachMembership $coach, array $slots): void
    {
        $this->entityManager->wrapInTransaction(function () use ($trainer, $coach, $slots): void {
            $this->windows->removeAllForCoach($coach);
            $this->entityManager->flush();

            foreach ($slots as $slot) {
                $this->windows->add(AvailabilityWindow::forCoach(
                    $trainer,
                    $coach,
                    $slot['dayOfWeek'],
                    $slot['startTime'],
                    $slot['endTime'],
                    $slot['isAvailable'],
                ));
            }
        });
    }

    /**
     * @return list<AvailabilityWindow>
     */
    public function getPlayerAvailability(PlayerProfile $player): array
    {
        return $this->windows->findForPlayer($player);
    }

    /**
     * @return list<AvailabilityWindow>
     */
    public function getCoachAvailability(CoachMembership $coach): array
    {
        return $this->windows->findForCoach($coach);
    }

    /**
     * AC-01-45: filter players by availability at a selected day/time.
     *
     * @return list<int>
     */
    public function playerIdsAvailableAt(int $dayOfWeek, \DateTimeImmutable $at): array
    {
        return $this->windows->findPlayerIdsAvailableAt($dayOfWeek, $at);
    }

    /**
     * AC-01-45: a per-player "Best Times" summary for the player card —
     * every available window, grouped by day, in a form a template can
     * render directly without re-deriving anything.
     *
     * @return array<int, list<AvailabilityWindow>> keyed by day of week (0-6)
     */
    public function bestTimesSummary(PlayerProfile $player): array
    {
        $summary = [];

        foreach ($this->windows->findForPlayer($player) as $window) {
            if ($window->isAvailable()) {
                $summary[$window->getDayOfWeek()][] = $window;
            }
        }

        return $summary;
    }
}
