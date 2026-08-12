<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\EmailVerificationToken;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<EmailVerificationToken>
 */
class EmailVerificationTokenRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, EmailVerificationToken::class);
    }

    public function findOneByTokenHash(string $tokenHash): ?EmailVerificationToken
    {
        return $this->findOneBy(['tokenHash' => $tokenHash]);
    }

    public function add(EmailVerificationToken $token): void
    {
        $this->getEntityManager()->persist($token);
    }
}
