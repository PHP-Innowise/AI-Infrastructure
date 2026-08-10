<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\ChildApprovalRequestRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * A child's RSVP or purchase attempt that is on hold pending a parent's
 * sign-off. Covers USD payments (BR-01-18, no bypass) and token purchases
 * (BR-01-19, bypassable per-child via ParentChildLink's toggle).
 *
 * `rsvpId`, `requestedTokenPackageId` and `requestedPlaylistId` are deferred
 * FKs by design: `rsvp`, `token_package` and `playlist` belong to Epic-02,
 * Epic-05 and Epic-04 respectively and do not exist yet. They are plain
 * nullable columns here, matching the schema's documented "deferred FK
 * attachment" pattern — the owning epic's own migration attaches the real
 * constraint once its target table exists.
 *
 * `status` deliberately has no "expired" value: BR-01-18's 48-hour window is
 * derived at read time (`isExpired()`), never materialized by a job.
 *
 * @see specs/database-designer-schema.md "`child_approval_request`"
 * @see specs/requirements-analyst-epic-01-user-management-spec.md BR-01-18..20, AC-01-25..28
 */
#[ORM\Entity(repositoryClass: ChildApprovalRequestRepository::class)]
#[ORM\Table(name: 'child_approval_request')]
#[ORM\Index(name: 'idx_car_parent_status', columns: ['parent_account_id', 'status'])]
#[ORM\Index(name: 'idx_car_child', columns: ['child_player_id'])]
#[TrainerScoped]
class ChildApprovalRequest
{
    public const ACTION_RSVP = 'rsvp';
    public const ACTION_RSVP_CANCELLATION = 'rsvp_cancellation';
    public const ACTION_TOKEN_PURCHASE = 'token_purchase';
    public const ACTION_CONTENT_PURCHASE = 'content_purchase';

    public const STATUS_PENDING = 'pending';
    public const STATUS_APPROVED = 'approved';
    public const STATUS_DENIED = 'denied';

    /**
     * BR-01-18: an unanswered request auto-expires after 48 hours.
     */
    private const EXPIRY_WINDOW = 'PT48H';

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'child_player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $childPlayer;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'parent_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $parentAccount;

    #[ORM\Column(name: 'action_type', type: 'string', length: 20)]
    private string $actionType;

    #[ORM\Column(name: 'rsvp_id', type: 'bigint', nullable: true)]
    private ?int $rsvpId = null;

    #[ORM\Column(name: 'requested_token_package_id', type: 'bigint', nullable: true)]
    private ?int $requestedTokenPackageId = null;

    #[ORM\Column(name: 'requested_playlist_id', type: 'bigint', nullable: true)]
    private ?int $requestedPlaylistId = null;

    #[ORM\Column(type: 'string', length: 16, options: ['default' => self::STATUS_PENDING])]
    private string $status = self::STATUS_PENDING;

    #[ORM\Column(name: 'requested_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $requestedAt;

    #[ORM\Column(name: 'responded_at', type: 'datetimetz_immutable', nullable: true)]
    private ?\DateTimeImmutable $respondedAt = null;

    #[ORM\Column(name: 'parent_note', type: 'text', nullable: true)]
    private ?string $parentNote = null;

    public function __construct(
        Trainer $trainer,
        PlayerProfile $childPlayer,
        Account $parentAccount,
        string $actionType,
        ?int $rsvpId = null,
        ?int $requestedTokenPackageId = null,
        ?int $requestedPlaylistId = null,
    ) {
        if (!\in_array($actionType, self::actionTypes(), true)) {
            throw new \InvalidArgumentException(sprintf('Unknown child approval action type "%s".', $actionType));
        }

        $this->trainer = $trainer;
        $this->childPlayer = $childPlayer;
        $this->parentAccount = $parentAccount;
        $this->actionType = $actionType;
        $this->rsvpId = $rsvpId;
        $this->requestedTokenPackageId = $requestedTokenPackageId;
        $this->requestedPlaylistId = $requestedPlaylistId;
        $this->requestedAt = new \DateTimeImmutable();
    }

    /**
     * @return list<string>
     */
    public static function actionTypes(): array
    {
        return [self::ACTION_RSVP, self::ACTION_RSVP_CANCELLATION, self::ACTION_TOKEN_PURCHASE, self::ACTION_CONTENT_PURCHASE];
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getChildPlayer(): PlayerProfile
    {
        return $this->childPlayer;
    }

    public function getParentAccount(): Account
    {
        return $this->parentAccount;
    }

    /**
     * The deferred-FK link to `rsvp` (see this class's own docblock) — added
     * by Epic-02's `RsvpService`, the first actual consumer of the
     * `requestApproval()` primitive this class's docblock names as the
     * hook Epic-02 would call.
     */
    public function getRsvpId(): ?int
    {
        return $this->rsvpId;
    }

    /**
     * The deferred-FK link to `playlist` (see this class's own docblock) —
     * added by Epic-04's `PurchasePlaylistAccessService`, mirroring
     * `getRsvpId()`'s own precedent exactly.
     */
    public function getRequestedPlaylistId(): ?int
    {
        return $this->requestedPlaylistId;
    }

    public function getActionType(): string
    {
        return $this->actionType;
    }

    public function getStatus(): string
    {
        return $this->status;
    }

    public function isPending(): bool
    {
        return self::STATUS_PENDING === $this->status;
    }

    public function getRequestedAt(): \DateTimeImmutable
    {
        return $this->requestedAt;
    }

    public function getRespondedAt(): ?\DateTimeImmutable
    {
        return $this->respondedAt;
    }

    public function getParentNote(): ?string
    {
        return $this->parentNote;
    }

    /**
     * Derived at read time, never materialized — architecture "Time-based
     * state is derived at read time".
     */
    public function isExpired(\DateTimeImmutable $now): bool
    {
        return $this->isPending() && $this->requestedAt->add(new \DateInterval(self::EXPIRY_WINDOW)) < $now;
    }

    /**
     * The status this row *reads as* right now — folds the derived "expired"
     * state into the four states the UI actually shows, without ever writing
     * one back (AC-01-25's own "read time, not a job" contract).
     */
    public function displayStatus(\DateTimeImmutable $now): string
    {
        return $this->isExpired($now) ? 'expired' : $this->status;
    }

    /**
     * AC-01-26, BR-01-20: approving processes payment/registration and
     * accepts an optional note.
     */
    public function approve(?string $note = null): void
    {
        $this->guardStillDecidable();

        $this->status = self::STATUS_APPROVED;
        $this->respondedAt = new \DateTimeImmutable();
        $this->parentNote = $note;
    }

    public function deny(?string $note = null): void
    {
        $this->guardStillDecidable();

        $this->status = self::STATUS_DENIED;
        $this->respondedAt = new \DateTimeImmutable();
        $this->parentNote = $note;
    }

    private function guardStillDecidable(): void
    {
        if (!$this->isPending()) {
            throw new \LogicException('This request has already been decided.');
        }
    }
}
