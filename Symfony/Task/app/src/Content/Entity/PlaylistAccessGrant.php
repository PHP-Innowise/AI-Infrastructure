<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Content\Repository\PlaylistAccessGrantRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\PlayerProfile;
use App\Platform\Entity\Trainer;
use App\Platform\Tenancy\TrainerScoped;
use Doctrine\ORM\Mapping as ORM;

/**
 * Records WHAT WAS BOUGHT for one player's access to one playlist (A8:
 * per-playlist one-time purchase), never a bare capability flag — so a
 * bundle or subscription pricing model can be layered on later (BR-04-8's
 * three other candidate models) without re-modelling this table, only
 * adding new grant-issuing paths that feed it.
 *
 * `paymentRecordId` is a deferred FK, matching the pattern established by
 * `App\Scheduling\Entity\Rsvp::$paymentRecordId` — `payment_record` belongs
 * to Epic-05/Billing, which does not exist yet. Epic-05's own migration
 * attaches the real constraint and, per
 * specs/database-designer-schema.md "Migration ordering" ("tightened from
 * nullable-during-creation to NOT NULL once the column is guaranteed
 * populated going forward"), tightens this column to NOT NULL at that point.
 * Kept nullable here rather than the schema's own per-table listing (which
 * shows it NOT NULL already, an internal inconsistency in that document —
 * see the coder's final report): a NOT NULL column would make it impossible
 * to ever construct this row before Epic-05 exists at all, which contradicts
 * shipping a real, testable grant-issuing code path behind a no-op payment
 * gateway (see `App\Content\Billing\NoopPaymentIntentGateway`).
 *
 * Access, once granted, persists forever (AC-05-18/BR-04-7) — there is no
 * revoke method. `(playlist, player)` is unique: a second purchase attempt
 * for an already-unlocked playlist is not a new grant.
 *
 * @see specs/database-designer-schema.md "`playlist_access_grant`"
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-6/7/8/9, A8
 */
#[ORM\Entity(repositoryClass: PlaylistAccessGrantRepository::class)]
#[ORM\Table(name: 'playlist_access_grant')]
#[ORM\UniqueConstraint(name: 'uniq_playlist_access_grant_playlist_player', columns: ['playlist_id', 'player_id'])]
#[ORM\Index(name: 'idx_playlist_access_grant_player', columns: ['player_id'])]
#[TrainerScoped]
class PlaylistAccessGrant
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\ManyToOne(targetEntity: Playlist::class)]
    #[ORM\JoinColumn(name: 'playlist_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Playlist $playlist;

    #[ORM\ManyToOne(targetEntity: PlayerProfile::class)]
    #[ORM\JoinColumn(name: 'player_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private PlayerProfile $player;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'parent_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $parentAccount;

    #[ORM\Column(name: 'payment_record_id', type: 'bigint', nullable: true)]
    private ?int $paymentRecordId = null;

    #[ORM\Column(name: 'granted_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $grantedAt;

    public function __construct(
        Trainer $trainer,
        Playlist $playlist,
        PlayerProfile $player,
        Account $parentAccount,
        ?int $paymentRecordId = null,
        ?\DateTimeImmutable $grantedAt = null,
    ) {
        $this->trainer = $trainer;
        $this->playlist = $playlist;
        $this->player = $player;
        $this->parentAccount = $parentAccount;
        $this->paymentRecordId = $paymentRecordId;
        $this->grantedAt = $grantedAt ?? new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getPlaylist(): Playlist
    {
        return $this->playlist;
    }

    public function getPlayer(): PlayerProfile
    {
        return $this->player;
    }

    public function getParentAccount(): Account
    {
        return $this->parentAccount;
    }

    public function getPaymentRecordId(): ?int
    {
        return $this->paymentRecordId;
    }

    public function getGrantedAt(): \DateTimeImmutable
    {
        return $this->grantedAt;
    }

    public function attachPaymentRecordId(int $paymentRecordId): void
    {
        $this->paymentRecordId = $paymentRecordId;
    }
}
