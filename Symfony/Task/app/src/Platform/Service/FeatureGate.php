<?php

declare(strict_types=1);

namespace App\Platform\Service;

use App\Platform\Entity\Trainer;
use App\Platform\Repository\FeatureToggleRepository;

/**
 * "Evaluate a trainer's feature toggles (LPPP, Marketing, Camps)"
 * (architect-architecture.md "Cross-cutting services", line 452) —
 * "consulted inside `CouponVoter`, the Growth voters, `FormVoter`, and the
 * Content voters, never expressible as a route prefix rule"
 * (security-voter-designer-design.md "Why the coarse gate is never the real
 * check"). A pure evaluator: it never denies anything itself, only answers
 * the question — the deny decision stays with whichever voter or controller
 * guard calls it.
 *
 * @see specs/architect-architecture.md "Cross-cutting services"
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md BR-07-1..3
 */
final readonly class FeatureGate
{
    public function __construct(
        private FeatureToggleRepository $toggles,
    ) {
    }

    /**
     * BR-07-2: "All three features default to enabled for new trainers."
     * A missing row means "never toggled" — enabled — not a third state;
     * `database-designer-schema.md`'s own note that `FeatureGate` "never
     * has to treat 'no row' as a third state" describes the steady state
     * once every trainer is seeded (`FeatureToggleRepository::seedDefaultsForTrainer()`
     * at trainer creation, and the Epic-07 migration's own backfill for
     * trainers created before this table existed) — this fallback is the
     * defensive second line for anything created outside that path (a
     * fixture, a `Trainer` built directly in a test), matching BR-07-2's
     * own default exactly rather than failing closed.
     */
    public function isEnabled(Trainer $trainer, string $featureName): bool
    {
        return $this->toggles->findOneByTrainerAndFeature($trainer, $featureName)?->isEnabled() ?? true;
    }
}
