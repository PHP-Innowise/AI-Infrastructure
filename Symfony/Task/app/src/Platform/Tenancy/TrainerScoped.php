<?php

declare(strict_types=1);

namespace App\Platform\Tenancy;

/**
 * Marks an entity as trainer-scoped: it carries a denormalized `trainer_id`
 * (tenancy layer 1), the Doctrine filter applies to it (layer 2), the
 * post-load assertion checks it (layer 4), and its table must appear in
 * config/tenancy/trainer_scoped_tables.txt with a Row-Level Security policy
 * (layer 5) or the container refuses to boot.
 *
 * Adding this attribute without adding the table to that manifest is a defect.
 */
#[\Attribute(\Attribute::TARGET_CLASS)]
final class TrainerScoped
{
}
