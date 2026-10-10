<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Billing\Entity\PaymentRecord;
use App\Billing\Repository\PaymentRecordRepository;
use App\Growth\Entity\Coupon;
use App\Growth\Entity\CouponRedemption;
use App\Growth\Entity\Referral;
use App\Growth\Entity\ReferralAssistCount;
use App\Growth\Entity\ReferralLink;
use App\Growth\Repository\CouponRedemptionRepository;
use App\Growth\Repository\CouponRepository;
use App\Growth\Repository\ReferralAssistCountRepository;
use App\Growth\Repository\ReferralLinkRepository;
use App\Growth\Repository\ReferralRepository;
use App\Identity\Entity\Account;
use App\Identity\Entity\AccountProfile;
use App\Identity\Entity\AccountRole;
use App\Identity\Entity\PlayerProfile;
use App\Identity\Entity\PlayerTrainerMembership;
use App\Identity\Service\MembershipService;
use App\Platform\Entity\PlatformConfiguration;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\PlatformConfigurationRepository;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Component\PasswordHasher\Hasher\UserPasswordHasherInterface;

/**
 * Growth-specific (Epic-06) fixture helpers, built on FixtureHelpers (the
 * base AppFixtures lookups, tenant activation) — the same "set up via the
 * entity/service layer, exercise the feature under test via HTTP" split
 * every other *FixtureHelpers trait in this suite already establishes.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`. The tenant must already be active
 * (`activateTenant($trainer)`) before calling any of these.
 */
trait GrowthFixtureHelpers
{
    /**
     * @param array{discountType?: string, discountValue?: int, appliesTo?: string, usageLimit?: ?int, eligibility?: string, expiresAt?: ?\DateTimeImmutable, isActive?: bool} $overrides
     */
    protected function createCoupon(Trainer $trainer, string $code, Account $createdBy, array $overrides = []): Coupon
    {
        $coupon = new Coupon(
            $trainer,
            $code,
            $overrides['discountType'] ?? Coupon::DISCOUNT_PERCENTAGE,
            $overrides['discountValue'] ?? 20,
            $overrides['appliesTo'] ?? Coupon::APPLIES_TO_BOTH,
            $overrides['usageLimit'] ?? null,
            $overrides['eligibility'] ?? Coupon::ELIGIBILITY_ANY_PLAYER,
            $overrides['expiresAt'] ?? null,
            $createdBy,
            $overrides['isActive'] ?? true,
        );

        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $coupons->add($coupon);
        $this->growthEntityManager()->flush();

        return $coupon;
    }

    protected function createReferralLink(Trainer $trainer, PlayerProfile $referrer): ReferralLink
    {
        $link = new ReferralLink($trainer, $referrer);

        /** @var ReferralLinkRepository $links */
        $links = self::getContainer()->get(ReferralLinkRepository::class);
        $links->add($link);
        $this->growthEntityManager()->flush();

        return $link;
    }

    protected function createReferral(
        Trainer $trainer,
        ReferralLink $link,
        PlayerProfile $referrer,
        PlayerProfile $referee,
        ?\DateTimeImmutable $clickedAt = null,
        ?\DateTimeImmutable $registeredAt = null,
    ): Referral {
        $referral = new Referral(
            $trainer,
            $link,
            $referrer,
            $referee,
            $clickedAt ?? new \DateTimeImmutable('-1 day'),
            $registeredAt ?? new \DateTimeImmutable(),
        );

        /** @var ReferralRepository $referrals */
        $referrals = self::getContainer()->get(ReferralRepository::class);
        $referrals->add($referral);
        $this->growthEntityManager()->flush();

        return $referral;
    }

    /**
     * A COMPLETED payment record, standing in for a real purchase —
     * `ReferralRewardService::processQualifyingPurchase()` and
     * `CouponPricingService::redeem()` both need a real, persisted
     * `PaymentRecord` to attach to (a converted `Referral`, a
     * `CouponRedemption`) but neither ever inspects `PaymentRecord::
     * getType()` — the type-specific dispatch (which of event_rsvp/
     * content_purchase/token_purchase actually happened) is
     * `ReferralRewardSubscriber`'s own job, exercised separately by the
     * full end-to-end HTTP+webhook tests in `CouponCheckoutTest`, not by
     * these more focused, service-level tests.
     *
     * Always `player_subscription`: the one type
     * `chk_payment_record_related_shape` requires NO related entity for
     * (`event_rsvp` requires a real `related_rsvp_id`, `content_purchase` a
     * real `related_playlist_id`, `token_purchase` a real
     * `related_token_package_id`) — constructing any of those just to
     * satisfy a CHECK constraint neither service under test ever reads
     * would be test setup noise standing in for nothing these tests
     * actually assert on.
     */
    protected function createCompletedPaymentRecord(Trainer $trainer, Account $payer, int $amountMinorUnits = 2000): PaymentRecord
    {
        $record = new PaymentRecord(
            $trainer,
            PaymentRecord::TYPE_PLAYER_SUBSCRIPTION,
            PaymentRecord::METHOD_CARD,
            $amountMinorUnits,
            'Test Payer',
            $payer->getEmail(),
            $payer,
        );
        // chk_payment_record_fee_shape: every non-refund record requires a
        // fee snapshot — matching what StripeGateway::applyCurrentFee()
        // would set on a real charge (BR-05-... "the fee rate in force is
        // recorded on every payment at creation").
        $record->applyFee(500, (int) round($amountMinorUnits * 500 / 10000));
        $record->markCompleted();

        /** @var PaymentRecordRepository $paymentRecords */
        $paymentRecords = self::getContainer()->get(PaymentRecordRepository::class);
        $paymentRecords->add($record);
        $this->growthEntityManager()->flush();

        return $record;
    }

    /**
     * A self-training adult player WITH a real login account, actively
     * associated with $trainer — unlike
     * `CrmFixtureHelpers::freshPlayerMembership()`, which creates an
     * account-less player (fine for CRM tests, useless for reward tests:
     * `PlayerAccountResolver::resolve()` needs a real account to credit
     * tokens to).
     *
     * @return array{account: Account, player: PlayerProfile}
     */
    protected function createPlayerWithAccount(Trainer $trainer, string $email): array
    {
        /** @var UserPasswordHasherInterface $hasher */
        $hasher = self::getContainer()->get(UserPasswordHasherInterface::class);
        $account = new Account($email, '', AccountRole::Player);
        $account->changePasswordHash($hasher->hashPassword($account, 'password'));
        $account->verifyEmail();
        $this->growthEntityManager()->persist($account);
        $this->growthEntityManager()->persist(new AccountProfile($account, 'Test', 'Player'));

        $player = new PlayerProfile('Test Player '.uniqid(), new \DateTimeImmutable('-25 years'), $account);
        $this->growthEntityManager()->persist($player);
        $this->growthEntityManager()->flush();

        /** @var MembershipService $membershipService */
        $membershipService = self::getContainer()->get(MembershipService::class);
        $membershipService->associatePlayer($trainer, $player, PlayerTrainerMembership::SOURCE_EVENT_REGISTRATION);

        return ['account' => $account, 'player' => $player];
    }

    /**
     * A direct CouponRedemption row, bypassing checkout — for tests whose
     * subject is the analytics READ side, not the checkout WRITE side
     * (`CouponCheckoutTest` already covers the full, real flow).
     * Increments the coupon's own usage count, matching what a real
     * redemption always does.
     */
    protected function createCouponRedemption(
        Trainer $trainer,
        Coupon $coupon,
        PlayerProfile $player,
        Account $payer,
        int $originalPriceMinorUnits,
        int $discountAmountMinorUnits,
        ?\DateTimeImmutable $redeemedAt = null,
    ): CouponRedemption {
        $payment = $this->createCompletedPaymentRecord($trainer, $payer, $originalPriceMinorUnits - $discountAmountMinorUnits);
        $redemption = new CouponRedemption($trainer, $coupon, $player, $payment, $originalPriceMinorUnits, $discountAmountMinorUnits, $originalPriceMinorUnits - $discountAmountMinorUnits);

        /** @var CouponRedemptionRepository $redemptions */
        $redemptions = self::getContainer()->get(CouponRedemptionRepository::class);
        $redemptions->add($redemption);
        $coupon->recordRedemption();
        $this->growthEntityManager()->flush();

        if (null !== $redeemedAt) {
            $reflection = new \ReflectionProperty(CouponRedemption::class, 'redeemedAt');
            $reflection->setValue($redemption, $redeemedAt);
            $this->growthEntityManager()->flush();
        }

        return $redemption;
    }

    protected function assistCountFor(Trainer $trainer, PlayerProfile $player): ?ReferralAssistCount
    {
        /** @var ReferralAssistCountRepository $repository */
        $repository = self::getContainer()->get(ReferralAssistCountRepository::class);

        return $repository->findOneByTrainerAndPlayer($trainer, $player);
    }

    /**
     * Sets the platform-wide referral rule directly, bypassing the
     * Super-Admin HTTP flow — for tests whose subject is something else
     * (the reward trigger, the dashboard) and only needs a deterministic
     * ratio to set up against.
     */
    protected function setReferralRule(int $referralsRequired, int $tokensAwarded, bool $refereeWelcomeBonus = false): void
    {
        /** @var PlatformConfigurationRepository $configuration */
        $configuration = self::getContainer()->get(PlatformConfigurationRepository::class);
        $configuration->upsert('referral_reward_ratio', ['referrals' => $referralsRequired, 'tokens' => $tokensAwarded], null);
        $configuration->upsert('referee_welcome_token', $refereeWelcomeBonus, null);
        $this->growthEntityManager()->flush();
    }

    protected function setReferralAttributionWindowDays(int $days): void
    {
        /** @var PlatformConfigurationRepository $configuration */
        $configuration = self::getContainer()->get(PlatformConfigurationRepository::class);
        $configuration->upsert('referral_attribution_window_days', $days, null);
        $this->growthEntityManager()->flush();
    }

    protected function platformConfigurationValue(string $key): mixed
    {
        /** @var PlatformConfigurationRepository $configuration */
        $configuration = self::getContainer()->get(PlatformConfigurationRepository::class);
        $row = $configuration->findOneByKey($key);

        self::assertInstanceOf(PlatformConfiguration::class, $row, sprintf('Expected a platform_configuration row for key "%s".', $key));

        return $row->getValue();
    }

    private function growthEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
