<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Entity\Coupon;
use App\Growth\Exception\CouponInUseException;
use App\Growth\Repository\CouponRepository;
use App\Growth\Service\CouponService;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.07 — Trainer Views Coupon Analytics.
 */
final class CouponAnalyticsTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-26: per coupon — code, discount, status, uses, usage limit,
     * discount given, revenue generated, conversion rate, dates.
     */
    public function testCouponListShowsAccuratePerCouponAnalytics(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $pat = $this->account('player@practiceperfect.test');

        $coupon = $this->createCoupon($trainer, 'ANALYZE1', $trainerAccount, [
            'discountType' => Coupon::DISCOUNT_FIXED,
            'discountValue' => 500,
            'usageLimit' => 4,
        ]);

        $this->createCouponRedemption($trainer, $coupon, $this->patPlayer(), $pat, 2000, 500);
        $this->createCouponRedemption($trainer, $coupon, $this->patPlayer(), $pat, 3000, 500);

        $this->client->loginUser($trainerAccount);
        $this->client->request('GET', '/trainer/marketing/coupons');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'ANALYZE1');
        self::assertSelectorTextContains('body', '$10.00', 'AC-06-26: total discount given ($5 + $5).');
        self::assertSelectorTextContains('body', '$40.00', 'AC-06-26: revenue generated ($15 + $25 final prices — $20-$5 and $30-$5).');
        self::assertSelectorTextContains('body', '50%', 'AC-06-26: conversion rate (2 uses / 4 limit).');
        self::assertSelectorTextContains('body', 'Active');
    }

    /**
     * AC-06-28: "Analytics Summary (all coupons combined)."
     */
    public function testAnalyticsSummaryAggregatesAcrossAllCoupons(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $pat = $this->account('player@practiceperfect.test');

        $active = $this->createCoupon($trainer, 'SUMMARY1', $trainerAccount);
        $this->createCoupon($trainer, 'SUMMARY2', $trainerAccount, ['isActive' => false]);
        $this->createCouponRedemption($trainer, $active, $this->patPlayer(), $pat, 1000, 200);

        $this->client->loginUser($trainerAccount);
        $this->client->request('GET', '/trainer/marketing/coupons');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '2', 'AC-06-28: total coupons created.');
        self::assertSelectorTextContains('body', '$2.00', 'AC-06-28: discount given this month.');
        self::assertSelectorTextContains('body', '$8.00', 'AC-06-28: revenue from coupon users this month.');
    }

    /**
     * AC-06-27: "view usage details (the list of players who used it)."
     */
    public function testTrainerCanViewCouponUsageDetails(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $pat = $this->account('player@practiceperfect.test');

        $coupon = $this->createCoupon($trainer, 'USAGEVIEW', $trainerAccount);
        $this->createCouponRedemption($trainer, $coupon, $this->patPlayer(), $pat, 2000, 400);

        $this->client->loginUser($trainerAccount);
        $this->client->request('GET', sprintf('/trainer/marketing/coupons/%d/usage', $coupon->getId()));

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', $this->patPlayer()->getFirstName(), 'AC-06-27: the list of players who used it.');
    }

    /**
     * AC-06-27: deleting a coupon that has already been used is blocked —
     * this codebase's own resolution of the epic's unstated edge case (see
     * `CouponInUseException`'s own docblock).
     */
    public function testDeletingAnAlreadyUsedCouponIsBlocked(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $pat = $this->account('player@practiceperfect.test');

        $coupon = $this->createCoupon($trainer, 'USEDCODE1', $trainerAccount);
        $this->createCouponRedemption($trainer, $coupon, $this->patPlayer(), $pat, 2000, 400);

        /** @var CouponService $couponService */
        $couponService = self::getContainer()->get(CouponService::class);

        $this->expectException(CouponInUseException::class);
        $couponService->delete($coupon);
    }

    /**
     * AC-06-27: deleting a coupon that has NEVER been used succeeds — the
     * positive control for the test above.
     */
    public function testDeletingAnUnusedCouponSucceeds(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $coupon = $this->createCoupon($trainer, 'NEVERUSED', $trainerAccount);
        $couponId = $coupon->getId();

        $this->client->loginUser($trainerAccount);
        // The CSRF token must come from a rendered page (session-bound),
        // never generated ad hoc via the container.
        $indexCrawler = $this->client->request('GET', '/trainer/marketing/coupons');
        $deleteForm = $indexCrawler->filter(sprintf('form[action$="/coupons/%d/delete"]', $couponId))->form();
        $this->client->submit($deleteForm);

        self::assertResponseRedirects('/trainer/marketing/coupons');

        $this->activateTenant($trainer);
        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        self::assertNull($coupons->findOneByTrainerAndCode($trainer, 'NEVERUSED'), 'AC-06-27: the unused coupon is gone.');
    }
}
