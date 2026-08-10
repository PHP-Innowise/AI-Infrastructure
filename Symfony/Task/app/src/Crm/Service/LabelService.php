<?php

declare(strict_types=1);

namespace App\Crm\Service;

use App\Crm\Entity\Label;
use App\Crm\Entity\PlayerLabel;
use App\Crm\Exception\DuplicateLabelNameException;
use App\Crm\Repository\LabelRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Entity\Trainer;
use Doctrine\DBAL\Exception\UniqueConstraintViolationException;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Label CRUD (US-03.03) and applying/removing labels on a player — one
 * bounded context, matching how `RsvpService` owns its whole lifecycle
 * rather than splitting into several micro-services.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-3/4/5, AC-03-11..14, AC-03-28
 */
final readonly class LabelService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private LabelRepository $labels,
        private PlayerLabelRepository $playerLabels,
    ) {
    }

    /**
     * @throws DuplicateLabelNameException
     */
    public function create(Trainer $trainer, string $name, string $colorHex): Label
    {
        return $this->entityManager->wrapInTransaction(function () use ($trainer, $name, $colorHex): Label {
            // BR-03-3: case-insensitive pre-check for a clean form error; the
            // database's own unique index is the authoritative backstop under
            // concurrency, caught below.
            if (null !== $this->labels->findOneByTrainerAndNameCaseInsensitive($trainer, $name)) {
                throw DuplicateLabelNameException::forName($name);
            }

            $label = new Label($trainer, $name, $colorHex);
            $this->labels->add($label);

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw DuplicateLabelNameException::forName($name);
            }

            return $label;
        });
    }

    /**
     * AC-03-13: edit a label's name and/or color.
     *
     * @throws DuplicateLabelNameException
     */
    public function rename(Label $label, string $name, string $colorHex): void
    {
        $this->entityManager->wrapInTransaction(function () use ($label, $name, $colorHex): void {
            $existing = $this->labels->findOneByTrainerAndNameCaseInsensitive($label->getTrainer(), $name, excluding: $label);

            if (null !== $existing) {
                throw DuplicateLabelNameException::forName($name);
            }

            $label->rename($name, $colorHex);

            try {
                $this->entityManager->flush();
            } catch (UniqueConstraintViolationException) {
                throw DuplicateLabelNameException::forName($name);
            }
        });
    }

    /**
     * BR-03-5: removes the label from every player it was applied to —
     * `player_label.label_id`'s `ON DELETE CASCADE` IS that removal, not a
     * side effect to guard against separately (see PlayerLabel's own
     * docblock).
     */
    public function delete(Label $label): void
    {
        $this->labels->remove($label);
        $this->entityManager->flush();
    }

    /**
     * AC-03-12: idempotent — a label already applied to this player is a
     * no-op, matching "a player can have multiple labels" without ever
     * inserting a duplicate join row (`uniq_player_label_player_label` is
     * the backstop).
     */
    public function applyToPlayer(PlayerTrainerMembership $membership, Label $label, Account $actor): PlayerLabel
    {
        $existing = $this->playerLabels->findOneByPlayerAndLabel($membership->getPlayer(), $label);

        if (null !== $existing) {
            return $existing;
        }

        $playerLabel = new PlayerLabel($membership->getTrainer(), $membership->getPlayer(), $label, $actor);
        $this->playerLabels->add($playerLabel);
        $this->entityManager->flush();

        return $playerLabel;
    }

    /**
     * AC-03-28: click-to-remove — removes only this one player's
     * association, never the label itself (BR-03-4).
     */
    public function removeFromPlayer(PlayerLabel $playerLabel): void
    {
        $this->playerLabels->remove($playerLabel);
        $this->entityManager->flush();
    }
}
