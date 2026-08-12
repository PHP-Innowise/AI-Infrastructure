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
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\EventDispatcher\EventDispatcherInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * US-01.02: registration via a trainer's ShareLink. One account, one player
 * profile, one membership, one confirmation email — a single transaction.
 * Also US-08.05: the identical account/player creation shape, entered from a
 * camp `FormSubmission` instead (`registerViaCampConversion()`) — kept on
 * this same class rather than duplicated into `Forms` because the entities
 * being created (`Account`, `PlayerProfile`, `ParentChildLink`) are
 * `Identity`'s own, and the module map permits `Forms` to call `Identity`
 * "to convert," never to construct these directly itself.
 *
 * A1 ("every under-18 player is parent-managed... no independent 16-18
 * accounts") is applied identically in both entry points: the player's
 * date-of-birth always creates a PlayerProfile, and whether that profile is
 * the registrant's own (self-training adult) or a child's (parent acting on
 * their behalf) is decided by age alone, not by a separate form choice —
 * there is no independent-minor path to branch into.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.02, AC-01-9..12, BR-01-14, BR-01-28
 * @see specs/requirements-analyst-epic-08-forms-registration-spec.md US-08.05, AC-08-23..26, BR-08-15..18
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

    /**
     * US-08.05/AC-08-24: a camp/evaluation `FormSubmission` converting to a
     * full account. Structurally the same shape as
     * `registerViaShareLink()` — including A1's child/adult branch, since a
     * converting registrant may just as well be a parent completing this on
     * a child's behalf as an adult registering themselves, and Epic-08
     * states no reason that rule would differ here — sourced from a
     * `Trainer` and raw contact/DOB data instead of a `ShareLink`:
     * `PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION` (A3's fourth CRM
     * association source) instead of `SOURCE_SHARELINK`, no `ShareLink`
     * reference, and AC-08-12's confirmation email replaced by
     * `App\Forms\Service\FormsMailer::sendConfirmation()` having already run
     * at submission time — this method sends only the verification email.
     *
     * The caller (`App\Forms\Service\FormSubmissionConversionService`) is
     * responsible for BR-08-17/A5's own remaining follow-up — marking the
     * `FormSubmission` converted and reattaching its `PaymentRecord` to the
     * new account — once this transaction commits: `Identity` has no reason
     * to know about `FormSubmission` or `PaymentRecord` at all (module map:
     * `Identity` "May call: `Platform`" only).
     *
     * @throws DuplicateEmailException
     */
    public function registerViaCampConversion(
        Trainer $trainer,
        string $participantName,
        string $email,
        string $plainPassword,
        \DateTimeImmutable $playerDateOfBirth,
        ?string $playerGender,
    ): Account {
        if (null !== $this->accounts->findOneByEmail($email)) {
            throw DuplicateEmailException::forEmail($email);
        }

        [$accountFirstName, $accountLastName] = self::splitName($participantName);
        $registeredPlayer = null;

        $account = $this->entityManager->wrapInTransaction(function () use (
            $trainer,
            $accountFirstName,
            $accountLastName,
            $email,
            $plainPassword,
            $participantName,
            $playerDateOfBirth,
            $playerGender,
            &$registeredPlayer,
        ): Account {
            $account = new Account($email, '', AccountRole::Player);
            $account->changePasswordHash($this->passwordHasher->hashPassword($account, $plainPassword));
            $this->accounts->add($account);
            $this->entityManager->persist(new AccountProfile($account, $accountFirstName, $accountLastName, null));

            $isChild = $playerDateOfBirth->diff(new \DateTimeImmutable())->y < 18;

            $player = new PlayerProfile($participantName, $playerDateOfBirth, $isChild ? null : $account, $playerGender);
            $this->playerProfiles->add($player);
            $registeredPlayer = $player;

            if ($isChild) {
                $this->parentChildLinks->add(new ParentChildLink($account, $player));
            }

            $this->entityManager->flush();

            // BR-08-17: auto-assigned to the trainer.
            $this->membershipService->associatePlayer($trainer, $player, PlayerTrainerMembership::SOURCE_CAMP_REGISTRATION);
            $this->entityManager->flush();

            $this->emailVerification->sendVerification($account);

            return $account;
        });

        \assert(null !== $registeredPlayer && null !== $registeredPlayer->getId());
        $this->eventDispatcher->dispatch(new PlayerRegistered(
            (int) $account->getId(),
            (int) $registeredPlayer->getId(),
            (int) $trainer->getId(),
            new \DateTimeImmutable(),
        ));

        return $account;
    }

    /**
     * @return array{0: string, 1: string}
     */
    private static function splitName(string $fullName): array
    {
        $parts = preg_split('/\s+/', trim($fullName), 2) ?: [$fullName];

        return [$parts[0], $parts[1] ?? ''];
    }
}
