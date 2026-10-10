<?php

declare(strict_types=1);

namespace App\Crm\Service;

use App\Crm\Entity\PlayerNote;
use App\Crm\Exception\NoteEditWindowExpiredException;
use App\Crm\Repository\PlayerNoteRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Scheduling\Entity\Event;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Adds, edits, and deletes player notes — general (AC-03-19), per-event
 * trainer notes (AC-03-20), and coach session feedback (AC-03-46..49), all
 * the same `PlayerNote` row shape (see that entity's own docblock).
 *
 * **Edit vs. delete deliberately use different time rules.** BR-03-12/
 * AC-03-21: a trainer can edit their own note within 24 hours but delete it
 * "at any time" — no time limit on delete at all. AC-03-49 additionally
 * gives a coach the SAME 24-hour window for both editing and deleting their
 * own feedback (no asymmetry there). Because `specs/api-designer-spec.md`'s
 * route table names the SAME voter attribute (`PLAYER_NOTE_MANAGE`) for both
 * the trainer's edit and delete routes, the voter cannot see which action is
 * being attempted and therefore cannot safely enforce a 24-hour cutoff that
 * only applies to one of the two (see `PlayerVoter`'s own docblock) — this
 * service is where the edit-specific window is actually enforced, re-checked
 * independently rather than trusting only a read-side "editable" flag,
 * mirroring `ChildApprovalVoter::CHILD_APPROVAL_DECIDE`'s own precedent for
 * re-verifying a time-boundary at the point of the actual write.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-10/11/12, AC-03-19..21, AC-03-46..49
 */
final readonly class PlayerNoteService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlayerNoteRepository $notes,
    ) {
    }

    public function addGeneral(PlayerTrainerMembership $membership, Account $author, string $text): PlayerNote
    {
        $note = new PlayerNote($membership->getTrainer(), $membership->getPlayer(), PlayerNote::TYPE_GENERAL, $text, $author);
        $this->notes->add($note);
        $this->entityManager->flush();

        return $note;
    }

    /**
     * AC-03-20 (trainer, per-event note) and AC-03-46 (coach, session
     * feedback) alike — the caller's own role is what distinguishes them,
     * not this method.
     */
    public function addSessionNote(PlayerTrainerMembership $membership, Event $event, Account $author, string $text): PlayerNote
    {
        $note = new PlayerNote($membership->getTrainer(), $membership->getPlayer(), PlayerNote::TYPE_SESSION, $text, $author, $event);
        $this->notes->add($note);
        $this->entityManager->flush();

        return $note;
    }

    /**
     * @throws NoteEditWindowExpiredException
     */
    public function edit(PlayerNote $note, Account $actor, string $text, \DateTimeImmutable $now): void
    {
        if ($now > $note->editWindowExpiresAt()) {
            throw NoteEditWindowExpiredException::create();
        }

        $note->editText($text, $actor);
        $this->entityManager->flush();
    }

    /**
     * AC-03-21: no time limit on delete for a trainer's own note (unlike
     * edit, above) — see this class's own docblock.
     */
    public function delete(PlayerNote $note): void
    {
        $this->notes->remove($note);
        $this->entityManager->flush();
    }
}
