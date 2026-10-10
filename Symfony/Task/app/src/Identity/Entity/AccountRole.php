<?php

declare(strict_types=1);

namespace App\Identity\Entity;

/**
 * The four MVP roles. Fixed constants, not runtime-editable data: owner
 * decision A2 puts the User Role Editor out of MVP.
 *
 * Deliberately NOT arranged in a hierarchy. Symfony's role_hierarchy would let
 * ROLE_SUPER_ADMIN inherit the others, and an ownership voter would then pass
 * silently for a Super Admin holding no tenant — exactly the cross-tenant write
 * the architecture forbids. Every voter handles Super Admin explicitly instead.
 *
 * @see specs/architect-architecture.md "Authorization"
 * @see specs/requirements-analyst-open-questions.md Section A, decision A2
 */
enum AccountRole: string
{
    case SuperAdmin = 'super_admin';
    case Trainer = 'trainer';
    case Coach = 'coach';
    case Player = 'player';

    /**
     * The Symfony security role string. One role per account, never a list:
     * BR-01-7 gives each account exactly one role.
     */
    public function securityRole(): string
    {
        return 'ROLE_'.strtoupper($this->value);
    }

    public function label(): string
    {
        return match ($this) {
            self::SuperAdmin => 'Super Admin',
            self::Trainer => 'Trainer',
            self::Coach => 'Coach',
            self::Player => 'Player / Parent',
        };
    }
}
