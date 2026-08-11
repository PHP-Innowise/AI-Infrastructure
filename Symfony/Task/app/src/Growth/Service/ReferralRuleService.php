<?php

declare(strict_types=1);

namespace App\Growth\Service;

use App\Growth\Dto\ReferralRule;
use App\Identity\Entity\Account;
use App\Platform\Repository\PlatformConfigurationRepository;
use App\Platform\Service\AuditLogger;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Growth's own typed view over `PlatformConfiguration`'s three referral
 * keys — the table itself stays a generic key/value store (Platform's own),
 * this service is where the interpretation and the AC-06-29..31 edit
 * workflow live.
 *
 * @see specs/requirements-analyst-epic-06-marketing-growth-spec.md US-06.08, AC-06-29..31, BR-06-4
 */
final readonly class ReferralRuleService
{
    private const KEY_REWARD_RATIO = 'referral_reward_ratio';
    private const KEY_ATTRIBUTION_WINDOW = 'referral_attribution_window_days';
    private const KEY_REFEREE_WELCOME = 'referee_welcome_token';

    public function __construct(
        private EntityManagerInterface $entityManager,
        private PlatformConfigurationRepository $configuration,
        private AuditLogger $auditLogger,
    ) {
    }

    /**
     * Q-06.11 defaults (1:1, no referee bonus), Q-06.12 default (30 days) —
     * lazily seeded by Version20260811120000, or by `getOrCreate()` itself
     * if that row is ever missing.
     */
    public function current(): ReferralRule
    {
        $ratio = $this->configuration->getOrCreate(self::KEY_REWARD_RATIO, ['referrals' => 1, 'tokens' => 1])->getValue();
        $window = $this->configuration->getOrCreate(self::KEY_ATTRIBUTION_WINDOW, 30)->getValue();
        $welcome = $this->configuration->getOrCreate(self::KEY_REFEREE_WELCOME, false)->getValue();

        \assert(\is_array($ratio));

        return new ReferralRule(
            referralsRequired: (int) ($ratio['referrals'] ?? 1),
            tokensAwarded: (int) ($ratio['tokens'] ?? 1),
            refereeWelcomeBonus: (bool) $welcome,
            attributionWindowDays: (int) $window,
        );
    }

    /**
     * AC-06-29/30/31: applies to every trainer immediately (nothing here is
     * trainer-scoped — the very next `current()` call, from any trainer's
     * context, sees the new values), existing assist counts are untouched
     * (nothing about `ReferralAssistCount` rows changes here), and the
     * change is audit-logged with old and new values.
     *
     * Deliberately does not touch the attribution window (Q-06.12) — the
     * epic's own edit form (`ReferralRuleType`) covers only "Referrals
     * Required," "Tokens Awarded," and "Reward Referee Too."
     */
    public function update(int $referralsRequired, int $tokensAwarded, bool $refereeWelcomeBonus, Account $changedBy): ReferralRule
    {
        if ($referralsRequired <= 0) {
            throw new \InvalidArgumentException('Referrals required must be a positive integer.');
        }

        if ($tokensAwarded <= 0) {
            throw new \InvalidArgumentException('Tokens awarded must be a positive integer.');
        }

        return $this->entityManager->wrapInTransaction(function () use ($referralsRequired, $tokensAwarded, $refereeWelcomeBonus, $changedBy): ReferralRule {
            $before = $this->current();

            $this->configuration->upsert(self::KEY_REWARD_RATIO, ['referrals' => $referralsRequired, 'tokens' => $tokensAwarded], $changedBy);
            $this->configuration->upsert(self::KEY_REFEREE_WELCOME, $refereeWelcomeBonus, $changedBy);
            $this->entityManager->flush();

            $this->auditLogger->record($changedBy, 'platform_configuration.referral_rule_changed', 'PlatformConfiguration', null, null, [
                'before' => [
                    'referralsRequired' => $before->referralsRequired,
                    'tokensAwarded' => $before->tokensAwarded,
                    'refereeWelcomeBonus' => $before->refereeWelcomeBonus,
                ],
                'after' => [
                    'referralsRequired' => $referralsRequired,
                    'tokensAwarded' => $tokensAwarded,
                    'refereeWelcomeBonus' => $refereeWelcomeBonus,
                ],
            ]);
            $this->entityManager->flush();

            return $this->current();
        });
    }
}
