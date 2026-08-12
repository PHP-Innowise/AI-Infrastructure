<?php

declare(strict_types=1);

namespace App\Platform\Service;

/**
 * One place to generate a bearer token and its storable hash. Every
 * time-limited link in the platform (password reset, account setup, email
 * verification) stores only the SHA-256 hash and emails only the raw value —
 * the raw token exists nowhere durable.
 */
final class SecureTokenFactory
{
    public function generate(): GeneratedToken
    {
        $raw = bin2hex(random_bytes(32));

        return new GeneratedToken($raw, $this->hash($raw));
    }

    public function hash(string $rawToken): string
    {
        return hash('sha256', $rawToken);
    }
}
