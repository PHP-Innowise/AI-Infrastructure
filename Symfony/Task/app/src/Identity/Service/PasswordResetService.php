<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\PasswordResetToken;
use App\Identity\Exception\TokenNotUsableException;
use App\Identity\Repository\AccountRepository;
use App\Identity\Repository\PasswordResetTokenRepository;
use App\Platform\Service\SecureTokenFactory;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * BR-01-4, AC-01-66: request → email → confirm, end to end. Also backs
 * AC-01-3/4/5's account-setup flow, which is the identical protocol (prove
 * control of the email, then set a password) reached from a different entry
 * point — see PasswordResetToken's own docblock.
 */
final readonly class PasswordResetService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private AccountRepository $accounts,
        private PasswordResetTokenRepository $tokens,
        private SecureTokenFactory $tokenFactory,
        private UserPasswordHasherInterface $passwordHasher,
        private IdentityMailer $mailer,
    ) {
    }

    /**
     * Enumeration-safe by construction: the caller always sees the same "if
     * that email exists, a reset link is on its way" outcome, whether or not
     * an account was found — api-designer-spec's own stated default.
     */
    public function requestReset(string $email): void
    {
        $account = $this->accounts->findOneByEmail($email);

        if (null === $account) {
            return;
        }

        $token = $this->tokenFactory->generate();
        $this->tokens->add(new PasswordResetToken($account, $token->hash));
        $this->entityManager->flush();

        $this->mailer->sendPasswordReset($account, $token->raw);
    }

    /**
     * @throws TokenNotUsableException
     */
    public function consume(string $rawToken, string $newPassword): Account
    {
        $token = $this->tokens->findOneByTokenHash($this->tokenFactory->hash($rawToken));

        if (null === $token || !$token->isUsable(new \DateTimeImmutable())) {
            throw TokenNotUsableException::create();
        }

        $account = $token->getAccount();
        $account->changePasswordHash($this->passwordHasher->hashPassword($account, $newPassword));
        $token->consume();

        $this->entityManager->flush();

        return $account;
    }
}
