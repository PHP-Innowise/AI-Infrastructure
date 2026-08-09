<?php

declare(strict_types=1);

namespace App\Identity\Repository;

use App\Identity\Entity\Account;
use Doctrine\Bundle\DoctrineBundle\Repository\ServiceEntityRepository;
use Doctrine\Persistence\ManagerRegistry;

/**
 * @extends ServiceEntityRepository<Account>
 */
class AccountRepository extends ServiceEntityRepository
{
    public function __construct(ManagerRegistry $registry)
    {
        parent::__construct($registry, Account::class);
    }

    /**
     * Email is CITEXT, so this is case-insensitive at the database level
     * rather than by convention at each call site (BR-01-2).
     */
    public function findOneByEmail(string $email): ?Account
    {
        return $this->findOneBy(['email' => $email]);
    }
}
