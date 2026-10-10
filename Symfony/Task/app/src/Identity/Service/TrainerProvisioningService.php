<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PasswordResetToken;
use App\Identity\Exception\DuplicateEmailException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Repository\PasswordResetTokenRepository;
use App\Platform\Entity\AccountTrainerLink;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\AccountTrainerLinkRepository;
use App\Platform\Repository\FeatureToggleRepository;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Service\AuditLogger;
use App\Platform\Service\SecureTokenFactory;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;
use Symfony\Component\String\Slugger\SluggerInterface;

/**
 * US-01.01: Super Admin creates a trainer account. Owns the whole workflow —
 * account + profile + tenant registration + the trainer's own tenant link +
 * a setup-password token + the trainer's default static ShareLink + the
 * setup email + the audit trail — as one transaction.
 *
 * BR-01-13: only this service creates a Trainer-role account. There is no
 * self-registration path anywhere in the codebase for this role.
 *
 * @see specs/requirements-analyst-epic-01-user-management-spec.md US-01.01, AC-01-1..8, BR-01-13
 */
final readonly class TrainerProvisioningService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AccountRepository $accounts,
        private TrainerRepository $trainers,
        private AccountTrainerLinkRepository $accountTrainerLinks,
        private PasswordResetTokenRepository $passwordResetTokens,
        private UserPasswordHasherInterface $passwordHasher,
        private SecureTokenFactory $tokenFactory,
        private SluggerInterface $slugger,
        private TenantContext $tenantContext,
        private ShareLinkService $shareLinkService,
        private IdentityMailer $mailer,
        private AuditLogger $auditLogger,
        private FeatureToggleRepository $featureToggles,
    ) {
    }

    /**
     * @throws DuplicateEmailException AC-01-7/8: email must be unique, with a
     *                                  clear error rather than a raw constraint violation
     */
    public function createTrainer(
        Account $actingSuperAdmin,
        string $businessName,
        string $trainerFirstName,
        string $trainerLastName,
        string $email,
        ?string $phone,
    ): Trainer {
        // AC-01-7: checked up front, outside the transaction, so the caller
        // gets a clean domain exception rather than a unique-violation from
        // flush(). The unique index remains the authoritative guard under
        // concurrency (BR-01-2).
        if (null !== $this->accounts->findOneByEmail($email)) {
            throw DuplicateEmailException::forEmail($email);
        }

        return $this->entityManager->wrapInTransaction(function () use (
            $actingSuperAdmin,
            $businessName,
            $trainerFirstName,
            $trainerLastName,
            $email,
            $phone,
        ): Trainer {
            // AC-01-3: a temporary, unusable password — nobody can log in
            // with it. The setup link (below) is the only way in until the
            // trainer sets their own password.
            $account = new Account($email, '', AccountRole::Trainer);
            $account->changePasswordHash($this->passwordHasher->hashPassword($account, bin2hex(random_bytes(32))));
            $this->accounts->add($account);

            $profile = new AccountProfile($account, $trainerFirstName, $trainerLastName, $phone);
            $this->entityManager->persist($profile);

            $slug = $this->uniqueSlug($businessName);
            $trainer = new Trainer($account, $businessName, $slug);
            $this->trainers->add($trainer);

            // IDs are needed below (ShareLink, PublicTenantCode, audit log).
            $this->entityManager->flush();

            $this->accountTrainerLinks->add(new AccountTrainerLink($account, $trainer, AccountRole::Trainer->value));

            $setupToken = $this->tokenFactory->generate();
            $this->passwordResetTokens->add(new PasswordResetToken($account, $setupToken->hash));

            // AC-01-73: every trainer gets a static player ShareLink from
            // day one. Trainer-scoped, so the tenant must be active first.
            $this->tenantContext->activateFor($trainer);
            $this->shareLinkService->issueStaticPlayerLink($trainer, $account);

            // BR-07-2: "All three features default to enabled for new
            // trainers" — seeded here so FeatureGate never has to treat "no
            // row" as a third state for a trainer created through the
            // normal path (database-designer-schema.md "`feature_toggle`").
            // `FeatureToggle` lives in `Platform`, not `Administration` —
            // see its own docblock — so this is Identity writing directly
            // to a Platform-owned repository, the same precedent
            // `$this->accountTrainerLinks->add()` two lines above already
            // sets in this exact method.
            $this->featureToggles->seedDefaultsForTrainer($trainer, $actingSuperAdmin);

            $this->auditLogger->record(
                $actingSuperAdmin,
                'trainer_created',
                'trainer',
                $trainer->getId(),
                $trainer,
                ['business_name' => $businessName, 'email' => $email],
            );

            // AC-01-3/4: emailed after everything else succeeds.
            $this->mailer->sendTrainerSetupInvite($account, $setupToken->raw);

            return $trainer;
        });
    }

    private function uniqueSlug(string $businessName): string
    {
        $base = strtolower((string) $this->slugger->slug($businessName));
        $base = '' === $base ? 'trainer' : $base;
        $slug = $base;
        $suffix = 1;

        while ($this->trainers->slugExists($slug)) {
            ++$suffix;
            $slug = $base.'-'.$suffix;
        }

        return $slug;
    }
}
