<?php

declare(strict_types=1);

namespace App\Content\Entity;

use App\Billing\Entity\PaymentRecord;
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
 * `paymentRecord` was a deferred, nullable scalar FK before Epic-05 existed
 * (see git history) — Epic-05's own migration (Version20260811100000)
 * attaches the real constraint and tightens the column to NOT NULL, per
 * specs/database-designer-schema.md "Migration ordering", now that
 * `payment_record` exists and every grant-issuing path
 * (`PurchasePlaylistAccessService`) supplies one at construction.
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

    #[ORM\ManyToOne(targetEntity: PaymentRecord::class)]
    #[ORM\JoinColumn(name: 'payment_record_id', referencedColumnName: 'id', nullable: false, unique: true, onDelete: 'RESTRICT')]
    private PaymentRecord $paymentRecord;

    #[ORM\Column(name: 'granted_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $grantedAt;

    public function __construct(
        Trainer $trainer,
        Playlist $playlist,
        PlayerProfile $player,
        Account $parentAccount,
        PaymentRecord $paymentRecord,
        ?\DateTimeImmutable $grantedAt = null,
    ) {
        $this->trainer = $trainer;
        $this->playlist = $playlist;
        $this->player = $player;
        $this->parentAccount = $parentAccount;
        $this->paymentRecord = $paymentRecord;
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

    public function getPaymentRecord(): PaymentRecord
    {
        return $this->paymentRecord;
    }

    public function getGrantedAt(): \DateTimeImmutable
    {
        return $this->grantedAt;
    }
}
