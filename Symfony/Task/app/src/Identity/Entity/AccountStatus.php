<?php

declare(strict_types=1);

namespace App\Identity\Entity;

/**
 * BR-01-23 (deactivation is reversible and preserves records) and BR-01-24
 * (GDPR deletion anonymizes in place rather than removing the row, so every
 * foreign key pointing at the account still resolves).
 */
enum AccountStatus: string
{
    case Active = 'active';
    case Inactive = 'inactive';
    case Deleted = 'deleted';

    public function canLogIn(): bool
    {
        return self::Active === $this;
    }
}
