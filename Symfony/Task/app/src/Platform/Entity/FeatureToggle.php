<?php

declare(strict_types=1);

namespace App\Platform\Entity;

use App\Identity\Entity\Account;
use App\Platform\Repository\FeatureToggleRepository;
use Doctrine\ORM\Mapping as ORM;

/**
 * Platform configuration *about* a trainer, owned and written by Super
 * Admin — but itself global (BR-07-1/2/3, AC-07-18..21).
 *
 * **Module placement deviates from `specs/database-designer-schema.md`'s own
 * "Entity population" tag** ("`FeatureToggle` | Administration"). Recorded
 * here and in the coder's final report as a forced deviation, not a silent
 * rename: `specs/architect-architecture.md`'s own "Module map" states
 * `Administration` "May call: `Platform` plus every module's services" and
 * that nothing calls back into `Administration` — the dependency arrow is
 * one-way, `Administration` -> everyone else, never the reverse. But
 * `specs/security-voter-designer-design.md`'s own Decision table requires
 * `FeatureGate::isEnabled($trainer, 'lppp')` to be "consulted inside
 * `CouponVoter`, the Growth voters, `FormVoter`, and the Content voters" —
 * i.e. `Content` and `Growth`, neither of which may depend on
 * `Administration` per the module map's own "May call" column. Placing
 * `FeatureToggle`/`FeatureGate` in `Platform` (the one module every other
 * module already depends on) is what makes both statements simultaneously
 * true, and matches the precedent already set by `AuditLogEntry` and
 * `ImpersonationSession` — both similarly Epic-07-flavored entities that the
 * Entity population table itself tags `Platform`, not `Administration`, for
 * exactly this reachability reason.
 *
 * No RLS: `trainer_id` here is a reference, not a tenancy key — this table
 * carries no policy (database-designer-schema.md "`feature_toggle`").
 *
 * @see specs/architect-architecture.md "Module map", "Entity population — Global"
 * @see specs/database-designer-schema.md "`feature_toggle` — Administration"
 * @see specs/security-voter-designer-design.md "Decisions" — FeatureGate row
 * @see specs/requirements-analyst-epic-07-super-admin-spec.md BR-07-1..3, AC-07-18..21
 */
#[ORM\Entity(repositoryClass: FeatureToggleRepository::class)]
#[ORM\Table(name: 'feature_toggle')]
#[ORM\UniqueConstraint(name: 'uniq_feature_toggle_trainer_feature', columns: ['trainer_id', 'feature_name'])]
#[ORM\Index(name: 'idx_feature_toggle_trainer', columns: ['trainer_id'])]
class FeatureToggle
{
    public const FEATURE_LPPP = 'lppp';
    public const FEATURE_MARKETING = 'marketing';
    public const FEATURE_CAMPS = 'camps';

    /**
     * AC-07-19: "Exactly three features are toggleable per trainer for
     * MVP." Order matters for display — matches the epic's own listing
     * order (LPPP, Marketing, Camps).
     *
     * @var list<string>
     */
    public const ALL_FEATURES = [self::FEATURE_LPPP, self::FEATURE_MARKETING, self::FEATURE_CAMPS];

    #[ORM\Id]
    #[ORM\GeneratedValue]
    #[ORM\Column(type: 'bigint')]
    private ?int $id = null;

    #[ORM\ManyToOne(targetEntity: Trainer::class)]
    #[ORM\JoinColumn(name: 'trainer_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Trainer $trainer;

    #[ORM\Column(name: 'feature_name', type: 'string', length: 16)]
    private string $featureName;

    /**
     * BR-07-2: defaults enabled.
     */
    #[ORM\Column(name: 'is_enabled', type: 'boolean', options: ['default' => true])]
    private bool $isEnabled = true;

    #[ORM\Column(name: 'updated_at', type: 'datetimetz_immutable')]
    private \DateTimeImmutable $updatedAt;

    #[ORM\ManyToOne(targetEntity: Account::class)]
    #[ORM\JoinColumn(name: 'updated_by_account_id', referencedColumnName: 'id', nullable: false, onDelete: 'RESTRICT')]
    private Account $updatedByAccount;

    public function __construct(Trainer $trainer, string $featureName, Account $updatedByAccount, bool $isEnabled = true)
    {
        if (!\in_array($featureName, self::ALL_FEATURES, true)) {
            throw new \InvalidArgumentException(sprintf('Unknown feature "%s".', $featureName));
        }

        $this->trainer = $trainer;
        $this->featureName = $featureName;
        $this->isEnabled = $isEnabled;
        $this->updatedByAccount = $updatedByAccount;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function getId(): ?int
    {
        return $this->id;
    }

    public function getTrainer(): Trainer
    {
        return $this->trainer;
    }

    public function getFeatureName(): string
    {
        return $this->featureName;
    }

    public function isEnabled(): bool
    {
        return $this->isEnabled;
    }

    public function getUpdatedAt(): \DateTimeImmutable
    {
        return $this->updatedAt;
    }

    public function getUpdatedByAccount(): Account
    {
        return $this->updatedByAccount;
    }

    /**
     * BR-07-3: takes effect immediately; disabling never deletes data —
     * nothing here touches any LPPP/Marketing/Camps row, only this flag.
     */
    public function enable(Account $by): void
    {
        $this->isEnabled = true;
        $this->updatedByAccount = $by;
        $this->updatedAt = new \DateTimeImmutable();
    }

    public function disable(Account $by): void
    {
        $this->isEnabled = false;
        $this->updatedByAccount = $by;
        $this->updatedAt = new \DateTimeImmutable();
    }

    /**
     * AC-07-18..20: a short display label for the toggle screen.
     */
    public function label(): string
    {
        return match ($this->featureName) {
            self::FEATURE_LPPP => 'LPPP Content System',
            self::FEATURE_MARKETING => 'Marketing Tools',
            self::FEATURE_CAMPS => 'Camps',
            default => $this->featureName,
        };
    }
}
