<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountRole;
use App\Identity\Repository\CoachMembershipRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\HttpFoundation\File\UploadedFile;

/**
 * US-01.11: "Profile"/"Account Settings" — the common fields every role
 * shares, plus each role's own extra fields applied on top (AC-01-51).
 * Email, role, and (for players) skill level are never accepted here — they
 * are simply never parameters of this method (AC-01-48).
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.11, AC-01-48..51
 */
final readonly class AccountProfileService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlayerProfileRepository $playerProfiles,
        private CoachMembershipRepository $coachMemberships,
        private string $uploadsDirectory,
        private string $uploadsPublicPath,
    ) {
    }

    /**
     * @param array{schoolOrTeam?: ?string, jerseyNumber?: ?string} $playerFields role-specific: Player
     * @param array{bio?: ?string, credentials?: ?string, certifications?: ?string, isPublicProfile?: bool} $coachFields role-specific: Coach
     * @param array{organizationAddress?: ?string, organizationWebsite?: ?string, organizationDescription?: ?string} $trainerFields role-specific: Trainer, applied to $trainerContext
     */
    public function updateProfile(
        Account $account,
        string $firstName,
        string $lastName,
        ?string $phone,
        ?string $schoolOrOrganization,
        ?UploadedFile $photo,
        array $playerFields = [],
        array $coachFields = [],
        array $trainerFields = [],
        ?Trainer $trainerContext = null,
    ): void {
        $this->entityManager->wrapInTransaction(function () use ($account, $firstName, $lastName, $phone, $schoolOrOrganization, $photo, $playerFields, $coachFields, $trainerFields, $trainerContext): void {
            $profile = $account->getProfile();

            if (null === $profile) {
                throw new \LogicException('An account being edited must already have a profile.');
            }

            $profile->updateCommonFields($firstName, $lastName, $phone, $schoolOrOrganization);

            if (null !== $photo) {
                $profile->updatePhoto($this->storePhoto($photo));
            }

            if (AccountRole::Player === $account->getRole()) {
                $this->applyPlayerFields($account, $playerFields);
            }

            if (AccountRole::Coach === $account->getRole()) {
                $this->applyCoachFields($account, $coachFields);
            }

            if (AccountRole::Trainer === $account->getRole() && null !== $trainerContext) {
                $trainerContext->updateOrganizationDetails(
                    $trainerFields['organizationAddress'] ?? null,
                    $trainerFields['organizationWebsite'] ?? null,
                    $trainerFields['organizationDescription'] ?? null,
                );
            }
        });
    }

    /**
     * @param array{schoolOrTeam?: ?string, jerseyNumber?: ?string} $fields
     */
    private function applyPlayerFields(Account $account, array $fields): void
    {
        if ([] === $fields) {
            return;
        }

        $player = $this->playerProfiles->findOneForSelfAccount($account);
        $player?->updateProfile(
            $player->getFirstName(),
            $player->getGender(),
            $fields['schoolOrTeam'] ?? $player->getSchoolOrTeam(),
            $fields['jerseyNumber'] ?? $player->getJerseyNumber(),
            $player->getEmergencyContactName(),
            $player->getEmergencyContactPhone(),
        );
    }

    /**
     * @param array{bio?: ?string, credentials?: ?string, certifications?: ?string, isPublicProfile?: bool} $fields
     */
    private function applyCoachFields(Account $account, array $fields): void
    {
        if ([] === $fields) {
            return;
        }

        $membership = $this->coachMemberships->findOneForAccountInActiveTenant($account);

        if (null === $membership) {
            return;
        }

        if (\array_key_exists('bio', $fields)) {
            $membership->setBio($fields['bio']);
        }

        if (\array_key_exists('credentials', $fields)) {
            $membership->setCredentials($fields['credentials']);
        }

        if (\array_key_exists('certifications', $fields)) {
            $membership->setCertifications($fields['certifications']);
        }

        if (\array_key_exists('isPublicProfile', $fields)) {
            $membership->setPublicProfile((bool) $fields['isPublicProfile']);
        }
    }

    private function storePhoto(UploadedFile $photo): string
    {
        $filename = bin2hex(random_bytes(16)).'.'.($photo->guessExtension() ?? 'bin');
        $photo->move($this->uploadsDirectory.'/profiles', $filename);

        return $this->uploadsPublicPath.'/profiles/'.$filename;
    }
}
