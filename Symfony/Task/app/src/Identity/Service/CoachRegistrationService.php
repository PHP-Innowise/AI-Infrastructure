<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\CoachMembership;
use App\Identity\Entity\ShareLink;
use App\Identity\Exception\CoachAlreadyActiveElsewhereException;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-01.08: a coach accepting a trainer's invite. Registering via the invite
 * link IS the acceptance — see MembershipService::acceptCoachInvite().
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.08, AC-01-39..41
 */
final readonly class CoachRegistrationService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AccountRepository $accounts,
        private UserPasswordHasherInterface $passwordHasher,
        private MembershipService $membershipService,
        private ShareLinkService $shareLinkService,
        private EmailVerificationService $emailVerification,
    ) {
    }

    /**
     * @throws DuplicateEmailException
     * @throws CoachAlreadyActiveElsewhereException BR-01-11/AC-01-41
     */
    public function registerViaInvite(
        ShareLink $invite,
        string $firstName,
        string $lastName,
        string $email,
        string $plainPassword,
    ): Account {
        if (null !== $this->accounts->findOneByEmail($email)) {
            throw DuplicateEmailException::forEmail($email);
        }

        return $this->entityManager->wrapInTransaction(function () use ($invite, $firstName, $lastName, $email, $plainPassword): Account {
            $account = new Account($email, '', AccountRole::Coach);
            $account->changePasswordHash($this->passwordHasher->hashPassword($account, $plainPassword));
            $this->accounts->add($account);
            $this->entityManager->persist(new AccountProfile($account, $firstName, $lastName));
            $this->entityManager->flush();

            // AC-01-40: throws CoachAlreadyActiveElsewhereException on
            // BR-01-11 violation — the whole registration rolls back, no
            // orphaned Account is left behind.
            $this->membershipService->acceptCoachInvite($invite->getTrainer(), $account, $invite, CoachMembership::STATUS_ACTIVE);
            $this->shareLinkService->recordUse($invite);
            $this->entityManager->flush();

            $this->emailVerification->sendVerification($account);

            return $account;
        });
    }
}
