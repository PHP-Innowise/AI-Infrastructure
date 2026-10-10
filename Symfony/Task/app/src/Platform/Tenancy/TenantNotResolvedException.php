<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

/**
 * Thrown when trainer-scoped data is reached with no tenant established.
 *
 * This is deliberately a hard failure rather than an empty result. RLS would
 * return zero rows and raise nothing, which reads exactly like "there is no
 * data" — the failure mode that lets a broken batch job process nothing,
 * forever, quietly.
 */
final class TenantNotResolvedException extends \RuntimeException
{
}
