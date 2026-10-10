<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Entity\Coupon;
use App\Growth\Repository\CouponRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.05 — Trainer Creates Coupon Code.
 */
final class CouponManagementTest extends WebTestCase
{
    use FixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-17/19: the full creation form (code, discount type/value,
     * applies-to, usage limit, expiration, status) — the coupon appears in
     * the trainer's list afterward.
     */
    public function testTrainerCreatesACouponWithAllFields(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/marketing/coupons/new');
        self::assertResponseIsSuccessful();

        $expires = (new \DateTimeImmutable('+60 days'))->format('Y-m-d\TH:i');
        $form = $crawler->selectButton('Create coupon')->form([
            'coupon[code]' => 'WELCOME99',
            'coupon[discountType]' => Coupon::DISCOUNT_PERCENTAGE,
            'coupon[discountValue]' => '20',
            'coupon[appliesTo]' => Coupon::APPLIES_TO_BOTH,
            'coupon[usageLimit]' => '50',
            'coupon[eligibility]' => Coupon::ELIGIBILITY_ANY_PLAYER,
            'coupon[expiresAt]' => $expires,
            'coupon[isActive]' => '1',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/marketing/coupons');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'WELCOME99', 'AC-06-19: the coupon appears in the trainer\'s coupon list.');

        $this->activateTenant($trainer);
        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $coupon = $coupons->findOneByTrainerAndCode($trainer, 'WELCOME99');
        self::assertNotNull($coupon);
        self::assertSame(20, $coupon->getDiscountValue());
        self::assertSame(Coupon::APPLIES_TO_BOTH, $coupon->getAppliesTo());
        self::assertSame(50, $coupon->getUsageLimit());
        self::assertTrue($coupon->isActive());
    }

    /**
     * AC-06-18: "the code must be unique within the trainer's own coupons."
     */
    public function testDuplicateCodeWithinTheSameTrainerIsRejected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->createCoupon($trainer, 'DUPE10', $this->account('trainer@practiceperfect.test'));

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/marketing/coupons/new');
        $form = $crawler->selectButton('Create coupon')->form([
            'coupon[code]' => 'DUPE10',
            'coupon[discountType]' => Coupon::DISCOUNT_PERCENTAGE,
            'coupon[discountValue]' => '10',
            'coupon[appliesTo]' => Coupon::APPLIES_TO_EVENTS,
            'coupon[eligibility]' => Coupon::ELIGIBILITY_ANY_PLAYER,
            'coupon[isActive]' => '1',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
        self::assertSelectorTextContains('body', 'already exists', 'AC-06-18: a clear duplicate-code error.');
    }

    /**
     * AC-06-18: the code must be 4-20 characters, alphanumeric plus
     * hyphens — enforced by `Coupon`'s own constructor guard.
     */
    public function testCodeFormatIsValidated(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $account = $this->account('trainer@practiceperfect.test');

        $this->expectException(\InvalidArgumentException::class);
        $this->expectExceptionMessage('AC-06-18');

        new Coupon($trainer, 'ab', Coupon::DISCOUNT_PERCENTAGE, 10, Coupon::APPLIES_TO_BOTH, null, Coupon::ELIGIBILITY_ANY_PLAYER, null, $account);
    }

    /**
     * AC-06-18: percentage discounts must be 1-100%.
     */
    public function testPercentageDiscountOver100IsRejected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $account = $this->account('trainer@practiceperfect.test');

        $this->expectException(\InvalidArgumentException::class);

        new Coupon($trainer, 'TOOMUCH20', Coupon::DISCOUNT_PERCENTAGE, 150, Coupon::APPLIES_TO_BOTH, null, Coupon::ELIGIBILITY_ANY_PLAYER, null, $account);
    }

    /**
     * AC-06-18: the discount value must be greater than 0.
     */
    public function testZeroDiscountValueIsRejected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $account = $this->account('trainer@practiceperfect.test');

        $this->expectException(\InvalidArgumentException::class);

        new Coupon($trainer, 'ZERODISC', Coupon::DISCOUNT_FIXED, 0, Coupon::APPLIES_TO_BOTH, null, Coupon::ELIGIBILITY_ANY_PLAYER, null, $account);
    }

    /**
     * AC-06-18: "an expiration date, if set, must be a future date."
     */
    public function testPastExpirationDateIsRejected(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $account = $this->account('trainer@practiceperfect.test');

        $this->expectException(\InvalidArgumentException::class);
        $this->expectExceptionMessage('AC-06-18');

        new Coupon($trainer, 'EXPIREDCODE', Coupon::DISCOUNT_PERCENTAGE, 10, Coupon::APPLIES_TO_BOTH, null, Coupon::ELIGIBILITY_ANY_PLAYER, new \DateTimeImmutable('-1 day'), $account);
    }

    /**
     * AC-06-19: the trainer can view, edit, or deactivate a coupon after
     * creation.
     */
    public function testTrainerCanEditAndDeactivateAnExistingCoupon(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $coupon = $this->createCoupon($trainer, 'EDITME1', $this->account('trainer@practiceperfect.test'), ['usageLimit' => 10]);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', sprintf('/trainer/marketing/coupons/%d/edit', $coupon->getId()));
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save changes')->form([
            'coupon[usageLimit]' => '25',
            'coupon[isActive]' => '1',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects('/trainer/marketing/coupons');

        $this->activateTenant($trainer);
        /** @var CouponRepository $coupons */
        $coupons = self::getContainer()->get(CouponRepository::class);
        $reloaded = $coupons->findOneByTrainerAndCode($trainer, 'EDITME1');
        self::assertNotNull($reloaded);
        self::assertSame(25, $reloaded->getUsageLimit(), 'AC-06-27: usage limit is editable.');

        // The CSRF token must come from a rendered page (session-bound),
        // never generated ad hoc via the container — the coupon index page
        // renders this exact deactivate form.
        $indexCrawler = $this->client->request('GET', '/trainer/marketing/coupons');
        $deactivateForm = $indexCrawler->filter(sprintf('form[action$="/coupons/%d/deactivate"]', $coupon->getId()))->form();
        $this->client->submit($deactivateForm);
        self::assertResponseRedirects('/trainer/marketing/coupons');

        // KernelBrowser reboots the kernel on every request — $coupons
        // above is bound to a now-discarded container/connection (a fresh
        // connection has no RLS session variable set, so re-using it here
        // would silently see nothing), so both the tenant activation AND
        // the repository are re-fetched fresh.
        $this->activateTenant($trainer);
        /** @var CouponRepository $freshCoupons */
        $freshCoupons = self::getContainer()->get(CouponRepository::class);
        $reloaded = $freshCoupons->findOneByTrainerAndCode($trainer, 'EDITME1');
        self::assertNotNull($reloaded);
        self::assertFalse($reloaded->isActive(), 'AC-06-19: the coupon is deactivated.');
    }

    /**
     * BR-06-11: a coupon created by Trainer A only works for Trainer A —
     * Trainer B cannot even reach it (cross-tenant isolation via RLS +
     * CouponVoter).
     */
    public function testTrainerCannotEditAnotherTrainersCoupon(): void
    {
        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $couponAId = $this->createCoupon($trainerA, 'TRAINERA1', $this->account('trainer@practiceperfect.test'))->getId();

        // Doctrine's identity map would otherwise still hand back the
        // PHP object created above from memory on the next find() — never
        // re-querying, so never re-applying RLS — since this test's setup
        // and the HTTP request below share one EntityManager. Clearing it
        // forces the controller's entity resolution to hit the database
        // fresh, under trainer B's own tenant context, exactly as a real,
        // separate request would.
        self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class)->clear();

        // Trainer B's own session.
        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/marketing/coupons/%d/edit', $couponAId));

        self::assertResponseStatusCodeSame(404, 'BR-06-11: cross-tenant coupon access is invisible (RLS), not merely denied.');
    }
}
