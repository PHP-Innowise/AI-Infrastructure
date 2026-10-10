<?php

declare(strict_types=1);

namespace App\Platform\Service;

/**
 * `raw` goes into the emailed link and nowhere else. `hash` is what gets
 * persisted and later compared against.
 */
final readonly class GeneratedToken
{
    public function __construct(
        public string $raw,
        public string $hash,
    ) {
    }
}
