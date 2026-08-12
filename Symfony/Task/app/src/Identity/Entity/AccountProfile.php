<?php

declare(strict_types=1);

namespace App\Identity\Entity;

use Doctrine\ORM\Mapping as ORM;

/**
 * The profile fields every role shares. Split from Account so the security
 * user object stays small and so GDPR anonymisation (BR-01-24) has one place
 * to null personal data without touching the row every foreign key points at.
 *
 * @see specs/database-designer-schema.md "`account_profile` — Identity"
 */
#[ORM\Entity]
#[ORM\Table(name: 'account_profile')]
class AccountProfile
{
    #[ORM\Id]
    #[ORM\OneToOne(inversedBy: 'profile', targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'account_id', referencedColumnName: 'id', onDelete: 'CASCADE')]
    private Account $account;

    #[ORM\Column(type: 'string', length: 100)]
    private string $firstName;

    #[ORM\Column(type: 'string', length: 100)]
    private string $lastName;

    #[ORM\Column(type: 'string', length: 32, nullable: true)]
    private ?string $phone = null;

    #[ORM\Column(type: 'string', length: 2048, nullable: true)]
    private ?string $photoUrl = null;

    #[ORM\Column(type: 'string', length: 255, nullable: true)]
    private ?string $schoolOrOrganization = null;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $createdAt;

    #[ORM\Column(type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    public function __construct(Account $account, string $firstName, string $lastName, ?string $phone = null)
    {
        $this->account = $account;
        $this->firstName = $firstName;
        $this->lastName = $lastName;
        $this->phone = $phone;
        $this->createdAt = new \DateTimeImmutable();
        $this->updatedAt = $this->createdAt;

        $account->setProfile($this);
    }

    public function getAccount(): Account
    {
        return $this->account;
    }

    public function getFirstName(): string
    {
        return $this->firstName;
    }

    public function getLastName(): string
    {
        return $this->lastName;
    }

    public function getFullName(): string
    {
        return $this->firstName.' '.$this->lastName;
    }

    public function getPhone(): ?string
    {
        return $this->phone;
    }

    public function getPhotoUrl(): ?string
    {
        return $this->photoUrl;
    }

    public function getSchoolOrOrganization(): ?string
    {
        return $this->schoolOrOrganization;
    }

    /**
     * AC-01-48/50: the common editable set for any role — first/last name,
     * phone. Email, role and (for players) skill level are never accepted
     * here; they are simply never bound from the edit form (AC-01-48).
     */
    public function updateCommonFields(string $firstName, string $lastName, ?string $phone, ?string $schoolOrOrganization): void
    {
        if ('' === trim($firstName) || '' === trim($lastName)) {
            throw new \InvalidArgumentException('First and last name are required.');
        }

        $this->firstName = $firstName;
        $this->lastName = $lastName;
        $this->phone = $phone;
        $this->schoolOrOrganization = $schoolOrOrganization;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * AC-01-49: an uploaded photo is stored in file storage with an updated
     * photo URL. (No thumbnail generation — recorded as a deliberate MVP
     * simplification, not silently dropped.)
     */
    public function updatePhoto(?string $photoUrl): void
    {
        $this->photoUrl = $photoUrl;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * BR-01-24: personal data is nulled in place. The row survives so that
     * historical records stay joinable while ceasing to identify anyone.
     */
    public function anonymize(): void
    {
        $this->firstName = 'Deleted';
        $this->lastName = 'User';
        $this->phone = null;
        $this->photoUrl = null;
        $this->schoolOrOrganization = null;
        $this->updatedAt = new \DateTimeImmutable();
    }
}
