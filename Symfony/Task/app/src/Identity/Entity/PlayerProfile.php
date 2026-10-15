<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use App\Identity\Repository\PlayerProfileRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * The person being trained — self or child — independent of any trainer.
 *
 * Global, because a player may train with several trainers and the person does
 * not belong to any one of them. Everything trainer-specific (skill level,
 * association source, status) lives on the trainer-scoped
 * PlayerTrainerMembership.
 *
 * Epic-01 states outright that it does not resolve how player and parent
 * should be modelled. This is the engineering resolution: `selfAccountId` is
 * the optional own-login link, and ParentChildLink is the parent relationship.
 * The business rule is already settled — owner decision A1 makes every
 * under-18 player parent-managed.
 *
 * @see specs/database-designer-schema.md "`player_profile` — Identity"
 */
#[ORM\Entity(repositoryClass: PlayerProfileRepository::class)]
#[ORM\Table(name: 'player_profile')]
#[ORM\UniqueConstraint(name: 'uniq_player_self_account', columns: ['self_account_id'])]
class PlayerProfile
{
    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'self_account_id', referencedColumnName: 'id', nullable: true, onDelete: 'RESTRICT')]
    private ?Account $selfAccount = null;

    #[ORM\Column(type: 'string', length: 100)]
    private string $firstName;

    /**
     * Date of birth is stored; age and age-group are derived at query time.
     * Q-01.02's safe default — storing a group would go stale every birthday.
     */
    #[ORM\Column(name: 'date_of_birth', type: 'date_immutable')]
    private \DateTimeImmutable $dateOfBirth;

    #[ORM\Column(type: 'string', length: 32, nullable: true)]
    private ?string $gender = null;

    /**
     * Denormalized and service-maintained, not database-generated: it depends
     * on ParentChildLink, a different table, and PostgreSQL generated columns
     * may only read the same row.
     */
    #[ORM\Column(name: 'is_child', type: 'boolean', options: ['default' => false])]
    private bool $isChild = false;

    /**
     * AC-01-16 (optional on child creation), AC-01-51 (Player role-specific
     * profile fields).
     */
    #[ORM\Column(name: 'school_or_team', type: 'string', length: 255, nullable: true)]
    private ?string $schoolOrTeam = null;

    #[ORM\Column(name: 'jersey_number', type: 'string', length: 16, nullable: true)]
    private ?string $jerseyNumber = null;

    /**
     * AC-01-51: Parent role-specific field, "if they have children" — stored
     * per child rather than once per parent, since a parent may reasonably
     * give a different emergency contact per child.
     */
    #[ORM\Column(name: 'emergency_contact_name', type: 'string', length: 255, nullable: true)]
    private ?string $emergencyContactName = null;

    #[ORM\Column(name: 'emergency_contact_phone', type: 'string', length: 32, nullable: true)]
    private ?string $emergencyContactPhone = null;

    #[ORM\Column(name: 'photo_url', type: 'string', length: 2048, nullable: true)]
    private ?string $photoUrl = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(
        string $firstName,
        \DateTimeImmutable $dateOfBirth,
        ?Account $selfAccount = null,
        ?string $gender = null,
    ) {
        if ('' === trim($firstName)) {
            throw new \InvalidArgumentException('A player profile requires a non-empty first name.');
        }

        $this->firstName = $firstName;
        $this->dateOfBirth = $dateOfBirth;
        $this->selfAccount = $selfAccount;
        $this->gender = $gender;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    /**
     * AC-01-20: a child profile can be given its own separate login after
     * the profile already exists — the constructor's `$selfAccount` only
     * covers the "known from creation" case (a self-registering adult).
     */
    public function attachSelfAccount(Account $account): void
    {
        if (null !== $this->selfAccount && $this->selfAccount !== $account) {
            throw new \LogicException('This player profile already has a different self-account attached.');
        }

        $this->selfAccount = $account;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function getFirstName(): string
    {
        return $this->firstName;
    }

    public function getSelfAccount(): ?Account
    {
        return $this->selfAccount;
    }

    public function getDateOfBirth(): \DateTimeImmutable
    {
        return $this->dateOfBirth;
    }

    /**
     * Signed, because `DateInterval::$y` is not.
     *
     * `diff()` reports the magnitude of the gap and puts its direction in
     * `$invert`, so reading `->y` alone made a date of birth in the future
     * come back as a positive age: a child entered as born in 2030 displayed
     * as "Age 3" and would have passed a minimum-age restriction on an event.
     * Manual testing found exactly that. A negative age is the honest answer
     * — visibly wrong on screen, and failing every `>= minAge` check rather
     * than sneaking past one.
     *
     * The forms reject a future date of birth outright (ChildProfileType,
     * PlayerRegistrationType), so this is the second line, not the first.
     */
    public function ageOn(\DateTimeImmutable $on): int
    {
        return self::ageInYears($this->dateOfBirth, $on);
    }

    /**
     * The same calculation before a profile exists — `ChildProfileService`
     * checks AC-01-21's 1-18 range against a submitted date, and had its own
     * copy of the `->y` reading, which is why a child "born" in 2030 passed a
     * range check that was working as written.
     */
    public static function ageInYears(\DateTimeImmutable $dateOfBirth, \DateTimeImmutable $on): int
    {
        $difference = $dateOfBirth->diff($on);

        return 1 === $difference->invert ? -$difference->y : $difference->y;
    }

    public function getGender(): ?string
    {
        return $this->gender;
    }

    public function getSchoolOrTeam(): ?string
    {
        return $this->schoolOrTeam;
    }

    public function getJerseyNumber(): ?string
    {
        return $this->jerseyNumber;
    }

    public function getEmergencyContactName(): ?string
    {
        return $this->emergencyContactName;
    }

    public function getEmergencyContactPhone(): ?string
    {
        return $this->emergencyContactPhone;
    }

    public function getPhotoUrl(): ?string
    {
        return $this->photoUrl;
    }

    public function isChild(): bool
    {
        return $this->isChild;
    }

    /**
     * Set when a ParentChildLink is created. Never legitimately flips back —
     * a child does not become un-parented by growing up; the account is
     * migrated instead.
     */
    public function markAsChild(): void
    {
        $this->isChild = true;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * AC-01-48/49/51: the editable subset of a player's own profile — name,
     * school/jersey/photo for a player; emergency contact for a parent acting
     * on a child. Age (date of birth), and therefore skill-derived fields,
     * stay out of this method deliberately: AC-01-48 keeps skill level
     * read-only, and date of birth is not named as editable anywhere in the
     * epic.
     */
    public function updateProfile(
        string $firstName,
        ?string $gender,
        ?string $schoolOrTeam,
        ?string $jerseyNumber,
        ?string $emergencyContactName,
        ?string $emergencyContactPhone,
    ): void {
        if ('' === trim($firstName)) {
            throw new \InvalidArgumentException('A player profile requires a non-empty first name.');
        }

        $this->firstName = $firstName;
        $this->gender = $gender;
        $this->schoolOrTeam = $schoolOrTeam;
        $this->jerseyNumber = $jerseyNumber;
        $this->emergencyContactName = $emergencyContactName;
        $this->emergencyContactPhone = $emergencyContactPhone;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function updatePhoto(?string $photoUrl): void
    {
        $this->photoUrl = $photoUrl;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
