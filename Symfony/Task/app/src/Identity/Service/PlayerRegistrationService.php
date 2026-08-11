<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\ParentChildLink;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Entity\ShareLink;
use App\Identity\Event\PlayerRegistered;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Repository\ParentChildLinkRepository;
use App\Identity\Repository\PlayerProfileRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\EventDispatcher\EventDispatcherInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-01.02: registration via a trainer's ShareLink. One account, one player
 * profile, one membership, one confirmation email — a single transaction.
 *
 * A1 ("every under-18 player is parent-managed... no independent 16-18
 * accounts") is applied here as the one rule this form needs: the "player
 * name/age/gender" fields (AC-01-10) always create a PlayerProfile, and
 * whether that profile is the registrant's own (self-training adult) or a
 * child's (parent registering on their behalf) is decided by age alone, not
 * by a separate form choice — there is no independent-minor path to branch
 * into.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.02, AC-01-9..12, BR-01-14, BR-01-28
 */
final readonly class PlayerRegistrationService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AccountRepository $accounts,
        private PlayerProfileRepository $playerProfiles,
        private ParentChildLinkRepository $parentChildLinks,
        private UserPasswordHasherInterface $passwordHasher,
        private MembershipService $membershipService,
        private ShareLinkService $shareLinkService,
        private EmailVerificationService $emailVerification,
        private IdentityMailer $mailer,
        private EventDispatcherInterface $eventDispatcher,
    ) {
    }

    /**
     * @throws DuplicateEmailException
     */
    public function registerViaShareLink(
        ShareLink $shareLink,
        string $accountFirstName,
        string $accountLastName,
        string $email,
        string $plainPassword,
        ?string $parentPhone,
        string $playerFirstName,
        \DateTimeImmutable $playerDateOfBirth,
        ?string $playerGender,
    ): Account {
        if (null !== $this->accounts->findOneByEmail($email)) {
            throw DuplicateEmailException::forEmail($email);
        }

        $registeredPlayer = null;

        $account = $this->entityManager->wrapInTransaction(function () use (
            $shareLink,
            $accountFirstName,
            $accountLastName,
            $email,
            $plainPassword,
            $parentPhone,
            $playerFirstName,
            $playerDateOfBirth,
            $playerGender,
            &$registeredPlayer,
        ): Account {
            $account = new Account($email, '', AccountRole::Player);
            $account->changePasswordHash($this->passwordHasher->hashPassword($account, $plainPassword));
            $this->accounts->add($account);
            $this->entityManager->persist(new AccountProfile($account, $accountFirstName, $accountLastName, $parentPhone));

            $isChild = $playerDateOfBirth->diff(new \DateTimeImmutable())->y < 18;

            $player = new PlayerProfile($playerFirstName, $playerDateOfBirth, $isChild ? null : $account, $playerGender);
            $this->playerProfiles->add($player);
            $registeredPlayer = $player;

            if ($isChild) {
                $this->parentChildLinks->add(new ParentChildLink($account, $player));
            }

            $this->entityManager->flush();

            // AC-01-11: auto-associated with the trainer who sent the link,
            // a player profile in that trainer's CRM.
            $this->membershipService->associatePlayer($shareLink->getTrainer(), $player, PlayerTrainerMembership::SOURCE_SHARELINK, $shareLink);
            $this->shareLinkService->recordUse($shareLink);
            $this->entityManager->flush();

            $this->emailVerification->sendVerification($account);
            // AC-01-12.
            $this->mailer->sendRegistrationConfirmation($account, $shareLink->getTrainer());

            return $account;
        });

        // Epic-06: dispatched AFTER commit — see PlayerRegistered's own
        // docblock for why this runs post-transaction, and why Identity
        // dispatches a plain event here rather than calling into Growth
        // directly (the module map permits Identity to call only Platform).
        \assert(null !== $registeredPlayer && null !== $registeredPlayer->getId());
        $this->eventDispatcher->dispatch(new PlayerRegistered(
            (int) $account->getId(),
            (int) $registeredPlayer->getId(),
            (int) $shareLink->getTrainer()->getId(),
            new \DateTimeImmutable(),
        ));

        return $account;
    }
}
