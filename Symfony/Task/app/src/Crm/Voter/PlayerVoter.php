<?php

declare(strict_types=1);

namespace App\Crm\Voter;

use App\Crm\Entity\PlayerNote;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Repository\CoachMembershipRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\AdministrativeScope;
use App\Scheduling\Service\CoachVisibilityService;
use Symfony\Component\Security\Core\Authentication\Token\TokenInterface;
use Symfony\Component\Security\Core\Authorization\Voter\Voter;

/**
 * BR-03-1/2/6/7/10/11/12, AC-03-1..49, AC-03-64/65.
 *
 * The subject is a `PlayerTrainerMembership` for every "act on this player
 * generally" attribute (`PLAYER_VIEW`, `PLAYER_EDIT`, `PLAYER_LABEL_MANAGE`,
 * `PLAYER_FLAG_MANAGE`, `PLAYER_FEEDBACK_ADD`), and additionally accepts a
 * `PlayerNote` for `PLAYER_NOTE_MANAGE` (editing/deleting one SPECIFIC note
 * needs its own author/timestamp, which a bare membership cannot carry) and
 * exclusively for `PLAYER_FEEDBACK_EDIT` (there is no "add" case to gate on
 * a membership for — see `PLAYER_FEEDBACK_ADD` instead). This is a wider
 * `supports()` than `specs/security-voter-designer-design.md`'s own summary
 * table literally states (it names `PlayerTrainerMembership` alone for every
 * attribute); the note-editing behavior that same document's prose demands
 * ("denies after 24h and denies on a coach-authored note") cannot be
 * expressed against a bare membership, so this is a necessary completion of
 * an under-specified subject shape, not a deviation from it — recorded in
 * the coder's final report.
 *
 * **Super Admin.** Per `specs/security-voter-designer-design.md` "Crm
 * module": `PLAYER_VIEW`, `PLAYER_FLAG_MANAGE`, `PLAYER_NOTE_MANAGE`,
 * `PLAYER_FEEDBACK_EDIT` each grant an explicit Super Admin override
 * (AC-03-56/58, AC-03-49, BR-03-12); `PLAYER_EDIT`, `PLAYER_LABEL_MANAGE`
 * and `PLAYER_FEEDBACK_ADD` do not (no route needs it, and — for
 * `PLAYER_LABEL_MANAGE` specifically — AC-03-58 states a deliberate
 * *prohibition*, matching `LabelVoter::LABEL_MANAGE`'s own absence). The
 * write MECHANISM for the four granted attributes is left open by that same
 * document ("cannot state whether it gates on `AdministrativeScope` or on
 * impersonation" — Open questions #1); this design resolves it the same way
 * `AttendanceVoter` already resolved the structurally identical gap for
 * BR-02-18's Super Admin branch: `AdministrativeScope`, for consistency
 * within this codebase rather than inventing a second mechanism.
 *
 * **Coach reach** delegates entirely to `CoachVisibilityService` — never
 * re-derived from `Rsvp`/`AttendanceRecord` here (architect-architecture.md
 * "a voter restating a visibility rule is a defect"). AC-03-45: reach is
 * necessary but never sufficient for a coach-authored WRITE — coaches never
 * reach `PLAYER_EDIT` or `PLAYER_LABEL_MANAGE` at all, matching AC-03-45's
 * "cannot edit player profiles except feedback... cannot remove labels or
 * flags".
 *
 * **`PLAYER_FLAG_MANAGE`'s coach branch is Q-03.05's on/off switch**,
 * default-denied per `specs/requirements-analyst-open-questions.md` Section
 * B ("Trainers only") until that question resolves — flip
 * `COACHES_MAY_MANAGE_FLAGS` once it does; no route or template change is
 * needed either way (`specs/api-designer-spec.md:539-546`).
 *
 * @see specs/security-voter-designer-design.md "Crm module"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md
 */
/**
 * @extends Voter<string, PlayerTrainerMembership|PlayerNote|null>
 */
final class PlayerVoter extends Voter
{
    public const PLAYER_VIEW = 'PLAYER_VIEW';
    public const PLAYER_EDIT = 'PLAYER_EDIT';
    public const PLAYER_LABEL_MANAGE = 'PLAYER_LABEL_MANAGE';
    public const PLAYER_FLAG_MANAGE = 'PLAYER_FLAG_MANAGE';
    public const PLAYER_NOTE_MANAGE = 'PLAYER_NOTE_MANAGE';
    public const PLAYER_FEEDBACK_ADD = 'PLAYER_FEEDBACK_ADD';
    public const PLAYER_FEEDBACK_EDIT = 'PLAYER_FEEDBACK_EDIT';

    /**
     * Q-03.05, unresolved upstream — default "Trainers only".
     */
    private const COACHES_MAY_MANAGE_FLAGS = false;

    public function __construct(
        private readonly CoachMembershipRepository $coachMemberships,
        private readonly CoachVisibilityService $coachVisibility,
        private readonly AdministrativeScope $administrativeScope,
    ) {
    }

    protected function supports(string $attribute, mixed $subject): bool
    {
        return match ($attribute) {
            self::PLAYER_VIEW, self::PLAYER_EDIT, self::PLAYER_LABEL_MANAGE, self::PLAYER_FLAG_MANAGE, self::PLAYER_FEEDBACK_ADD =>
                $subject instanceof PlayerTrainerMembership,
            self::PLAYER_NOTE_MANAGE => $subject instanceof PlayerTrainerMembership || $subject instanceof PlayerNote,
            self::PLAYER_FEEDBACK_EDIT => $subject instanceof PlayerNote,
            default => false,
        };
    }

    protected function voteOnAttribute(string $attribute, mixed $subject, TokenInterface $token): bool
    {
        $actor = $token->getUser();

        if (!$actor instanceof Account) {
            return false;
        }

        [$trainer, $player, $note] = $this->resolveSubject($subject);

        return match ($attribute) {
            self::PLAYER_VIEW => $this->voteView($trainer, $player, $actor),
            // AC-03-33: trainer-only; structurally same-tenant already.
            self::PLAYER_EDIT => AccountRole::Trainer === $actor->getRole(),
            // AC-03-53: the coach who invites a player can never apply
            // labels to them — trainer-only, no coach branch at all.
            self::PLAYER_LABEL_MANAGE => AccountRole::Trainer === $actor->getRole(),
            self::PLAYER_FLAG_MANAGE => $this->voteFlagManage($trainer, $player, $actor),
            self::PLAYER_NOTE_MANAGE => $this->voteNoteManage($trainer, $actor, $note),
            self::PLAYER_FEEDBACK_ADD => $this->voteFeedbackAdd($player, $actor),
            self::PLAYER_FEEDBACK_EDIT => $this->voteFeedbackEdit($note, $actor),
            default => false,
        };
    }

    private function voteView(Trainer $trainer, PlayerProfile $player, Account $actor): bool
    {
        if (AccountRole::Trainer === $actor->getRole()) {
            // Structurally same-tenant already: RLS + the Doctrine filter
            // make a foreign-tenant membership invisible before this voter
            // ever runs.
            return true;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()) {
            return $this->administrativeScope->isOpenFor($trainer);
        }

        if (AccountRole::Coach !== $actor->getRole()) {
            return false;
        }

        $coach = $this->coachMemberships->findOneForAccountInActiveTenant($actor);

        // AC-03-44's note/architect-architecture.md Open architecture risk
        // 6: a player with no shared session history is unreachable, not an
        // empty profile.
        return null !== $coach && $this->coachVisibility->canReachPlayer($coach, $player);
    }

    private function voteFlagManage(Trainer $trainer, PlayerProfile $player, Account $actor): bool
    {
        if (AccountRole::Trainer === $actor->getRole()) {
            return true;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()) {
            return $this->administrativeScope->isOpenFor($trainer);
        }

        // Q-03.05 default: "Trainers only" — COACHES_MAY_MANAGE_FLAGS is
        // false today by design, which makes this branch provably
        // unreachable code to PHPStan's eyes right now. It stays written
        // out, not deleted, because Q-03.05 flipping the constant to true
        // is the one-line change this whole method exists to make trivial
        // once that question resolves — see this class's own docblock.
        // @phpstan-ignore booleanNot.alwaysTrue, booleanOr.alwaysTrue
        if (!self::COACHES_MAY_MANAGE_FLAGS || AccountRole::Coach !== $actor->getRole()) {
            return false;
        }

        // @phpstan-ignore deadCode.unreachable
        $coach = $this->coachMemberships->findOneForAccountInActiveTenant($actor);

        return null !== $coach && $this->coachVisibility->canReachPlayer($coach, $player);
    }

    private function voteNoteManage(Trainer $trainer, Account $actor, ?PlayerNote $note): bool
    {
        if (AccountRole::SuperAdmin === $actor->getRole()) {
            return $this->administrativeScope->isOpenFor($trainer);
        }

        // BR-03-10: general/per-event TRAINER notes only — coach writes go
        // through PLAYER_FEEDBACK_ADD/_EDIT onto the same PlayerNote table,
        // never through this branch.
        if (AccountRole::Trainer !== $actor->getRole()) {
            return false;
        }

        if (null === $note) {
            // Adding a new note: structurally same-tenant already.
            return true;
        }

        // AC-03-21/BR-03-12: read-only to the trainer once authored by a
        // coach — never editable or deletable by the trainer, at any time.
        // The 24-hour EDIT-only window is enforced by PlayerNoteService, not
        // here — see that service's own docblock for why edit and delete
        // cannot share one time rule under this single attribute.
        return AccountRole::Coach !== $note->getCreatedByAccount()->getRole();
    }

    private function voteFeedbackAdd(PlayerProfile $player, Account $actor): bool
    {
        if (AccountRole::Coach !== $actor->getRole()) {
            return false;
        }

        $coach = $this->coachMemberships->findOneForAccountInActiveTenant($actor);

        return null !== $coach && $this->coachVisibility->canReachPlayer($coach, $player);
    }

    private function voteFeedbackEdit(?PlayerNote $note, Account $actor): bool
    {
        if (null === $note) {
            return false;
        }

        if (AccountRole::SuperAdmin === $actor->getRole()) {
            // BR-03-12/AC-03-49: "at any time" — no window for Super Admin.
            return $this->administrativeScope->isOpenFor($note->getTrainer());
        }

        if (AccountRole::Coach !== $actor->getRole()) {
            return false;
        }

        // AC-03-49: own feedback only, within 24 hours — re-verified here
        // rather than trusted only from a read-side label, the same
        // "re-apply the cutoff at the point of decision" precedent
        // ChildApprovalVoter::CHILD_APPROVAL_DECIDE already established.
        return $actor === $note->getCreatedByAccount() && new \DateTimeImmutable() <= $note->editWindowExpiresAt();
    }

    /**
     * @return array{0: Trainer, 1: PlayerProfile, 2: ?PlayerNote}
     */
    private function resolveSubject(mixed $subject): array
    {
        if ($subject instanceof PlayerNote) {
            return [$subject->getTrainer(), $subject->getPlayer(), $subject];
        }

        \assert($subject instanceof PlayerTrainerMembership);

        return [$subject->getTrainer(), $subject->getPlayer(), null];
    }
}
