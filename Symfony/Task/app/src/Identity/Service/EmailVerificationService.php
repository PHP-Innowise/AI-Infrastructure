<?php

declare(strict_types=1);

namespace App\Identity\Service;

use App\Identity\Entity\Account;
use App\Identity\Entity\EmailVerificationToken;
use App\Identity\Exception\TokenNotUsableException;
use App\Identity\Repository\EmailVerificationTokenRepository;
use App\Platform\Service\SecureTokenFactory;
use Doctrine\ORM\EntityManagerInterface;

/**
 * BR-01-5, AC-01-67: send → process. Q-01.05 (whether verification blocks
 * login) is an open question this class does not resolve — see
 * `Account::isActive()`/the security authenticator, which gate on account
 * status only, never on `emailVerifiedAt`.
 */
final readonly class EmailVerificationService
{
    public function __construct(
        private EntityManagerInterface $entityManager,
        private EmailVerificationTokenRepository $tokens,
        private SecureTokenFactory $tokenFactory,
        private IdentityMailer $mailer,
    ) {
    }

    public function sendVerification(Account $account): void
    {
        $token = $this->tokenFactory->generate();
        $this->tokens->add(new EmailVerificationToken($account, $token->hash));
        $this->entityManager->flush();

        $this->mailer->sendEmailVerification($account, $token->raw);
    }

    /**
     * @throws TokenNotUsableException
     */
    public function verify(string $rawToken): Account
    {
        $token = $this->tokens->findOneByTokenHash($this->tokenFactory->hash($rawToken));

        if (null === $token || !$token->isUsable(new \DateTimeImmutable())) {
            throw TokenNotUsableException::create();
        }

        $account = $token->getAccount();
        $account->verifyEmail();
        $token->consume();

        $this->entityManager->flush();

        return $account;
    }
}
