<?php

declare(strict_types=1);

namespace App\Crm\Entity;

use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Crm\Repository\PlayerNoteRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use App\Scheduling\Entity\Event;
use Doctrine\ORM\Mapping as ORM;

/**
 * A note about a player — general (not tied to a session) or session-scoped.
 *
 * One entity backs THREE user-facing concepts, deliberately unified rather
 * than split, because the epic's own data requirements describe them with
 * the identical shape (note id, player, type, optional session, text,
 * author, timestamps — "Data requirements", line 628):
 *
 *  - AC-03-19 general notes: `noteType = general`, `event = null`, trainer-authored.
 *  - AC-03-20 per-event trainer notes: `noteType = session`, `event` set, trainer-authored.
 *  - AC-03-46..49 coach session feedback: `noteType = session`, `event` set,
 *    coach-authored. The epic itself never distinguishes "event" from
 *    "session" as attendance units (see the coder's final report) — coach
 *    "session feedback" and a trainer's "per-event note" are the same
 *    fact (a note tied to one attendance unit), differing only in who wrote
 *    it, which this row already records via `createdByAccount`.
 *
 * Authorization (creator-only edit within 24h, trainer cannot edit a coach
 * note, Super Admin can edit any note at any time — BR-03-12) lives in
 * `PlayerVoter`, not here, matching how `AttendanceVoter` keeps the same-day
 * edit window out of `AttendanceRecord` itself. The 24-hour EDIT window
 * specifically (as opposed to delete, which BR-03-10/AC-03-21 place no time
 * limit on for a trainer's own notes) is re-verified in
 * `PlayerNoteService::edit()` — see that method's own docblock for why edit
 * and delete cannot share one time rule.
 *
 * @see specs/database-designer-schema.md "`player_note`"
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md BR-03-10/11/12, AC-03-19..21, AC-03-46..49
 */
#[ORM\Entity(repositoryClass: PlayerNoteRepository::class)]
#[ORM\Table(name: 'player_note')]
#[ORM\Index(name: 'idx_player_note_player_created', columns: ['player_id', 'created_at'])]
#[ORM\Index(name: 'idx_player_note_event', columns: ['event_id'])]
#[TrainerScoped]
class PlayerNote
{
    public const TYPE_GENERAL = 'general';
    public const TYPE_SESSION = 'session';

    public const MAX_TEXT_LENGTH = 1000;

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\Column(name: 'note_type', type: 'string', length: 16)]
    private string $noteType;

    #[ORM\ManyToOne(targetEntity: Event::class)]
    #[ORM\JoinColumn(name: 'event_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Event $event = null;

    #[ORM\Column(name: 'note_text', type: 'string', length: self::MAX_TEXT_LENGTH)]
    private string $noteText;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'created_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $createdByAccount;

    #[ORM\Column(name: 'created_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $updatedAt = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'edited_by_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $editedByAccount = null;

    public function __construct(
        Trainer $trainer,
        PlayerProfile $player,
        string $noteType,
        string $noteText,
        Account $createdByAccount,
        ?Event $event = null,
    ) {
        self::guardType($noteType, $event);
        $this->guardText($noteText);

        $this->trainer = $trainer;
        $this->player = $player;
        $this->noteType = $noteType;
        $this->event = $event;
        $this->noteText = $noteText;
        $this->createdByAccount = $createdByAccount;
        $this->createdAt = new \DateTimeImmutable();
    }

    /**
     * @return list<string>
     */
    public static function types(): array
    {
        return [self::TYPE_GENERAL, self::TYPE_SESSION];
    }

    public static function guardType(string $noteType, ?Event $event): void
    {
        if (!\in_array($noteType, self::types(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown note type "%s".', $noteType));
        }

        if ((self::TYPE_SESSION === $noteType) !== (null !== $event)) {
            throw new \InvalidArgumentException('A session note requires an event; a general note must not carry one.');
        }
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getNoteType(): string
    {
        return $this->noteType;
    }

    public function isSessionNote(): bool
    {
        return self::TYPE_SESSION === $this->noteType;
    }

    public function getEvent(): ?Event
    {
        return $this->event;
    }

    public function getNoteText(): string
    {
        return $this->noteText;
    }

    public function getCreatedByAccount(): Account
    {
        return $this->createdByAccount;
    }

    public function getCreatedAt(): \DateTimeImmutable
    {
        return $this->createdAt;
    }

    public function getUpdatedAt(): ?\DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function getEditedByAccount(): ?Account
    {
        return $this->editedByAccount;
    }

    /**
     * BR-03-12/AC-03-21: the caller (PlayerVoter, PlayerNoteService) is
     * responsible for deciding whether this edit is currently permitted —
     * this only applies the new text and stamps who/when.
     */
    public function editText(string $noteText, Account $editedBy): void
    {
        $this->guardText($noteText);

        $this->noteText = $noteText;
        $this->editedByAccount = $editedBy;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * The precise instant BR-03-12's 24-hour edit window is measured from.
     */
    public function editWindowExpiresAt(): \DateTimeImmutable
    {
        return $this->createdAt->modify('+24 hours');
    }

    private function guardText(string $noteText): void
    {
        if ('' === trim($noteText)) {
            throw new \InvalidArgumentException('A note requires non-empty text.');
        }

        if (mb_strlen($noteText) > self::MAX_TEXT_LENGTH) {
            throw new \InvalidArgumentException(sprintf('A note cannot exceed %d characters.', self::MAX_TEXT_LENGTH));
        }
    }
}
