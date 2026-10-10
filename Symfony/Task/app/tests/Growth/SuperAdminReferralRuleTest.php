<?php

declare(strict_types=1);

namespace App\Tests\Growth;

use App\Growth\Service\ReferralRuleService;
use App\Platform\Repository\AuditLogEntryRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\GrowthFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-06.08 — Super Admin Configures Referral Reward Rules.
 */
final class SuperAdminReferralRuleTest extends WebTestCase
{
    use FixtureHelpers;
    use GrowthFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-06-29: views the current rule, edits Referrals Required, Tokens
     * Awarded, and the referee welcome-bonus toggle, and saves.
     */
    public function testSuperAdminViewsAndUpdatesTheReferralRule(): void
    {
        // `platform_configuration` is a genuinely global singleton — set
        // the starting rule explicitly rather than assuming the untouched
        // Q-06.11 default, since other tests in this same suite run also
        // exercise this same global row.
        $this->setReferralRule(1, 1, false);

        $superAdmin = $this->account('admin@practiceperfect.test');
        $this->client->loginUser($superAdmin);

        $crawler = $this->client->request('GET', '/super-admin/growth/referral-rules');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '1 referral purchase = 1 token', 'AC-06-29: current rule shown (e.g. "3 referral purchases = 1 token").');

        $form = $crawler->selectButton('Save rule')->form([
            'referral_rule[referralsRequired]' => '5',
            'referral_rule[tokensAwarded]' => '2',
            'referral_rule[refereeWelcomeBonus]' => '1',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/super-admin/growth/referral-rules');

        /** @var ReferralRuleService $referralRules */
        $referralRules = self::getContainer()->get(ReferralRuleService::class);
        $updated = $referralRules->current();
        self::assertSame(5, $updated->referralsRequired);
        self::assertSame(2, $updated->tokensAwarded);
        self::assertTrue($updated->refereeWelcomeBonus);
    }

    /**
     * AC-06-30: the new rule applies to every trainer immediately, and
     * existing per-player assist counts are preserved under it.
     */
    public function testUpdatingTheRuleAppliesImmediatelyAndPreservesExistingAssistCounts(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $this->setReferralRule(3, 1);

        $referrer = $this->createPlayerWithAccount($trainer, 'preserve-assists@example.test');
        $assistCount = new \App\Growth\Entity\ReferralAssistCount($trainer, $referrer['player']);
        $assistCount->increment();
        $assistCount->increment();
        self::getContainer()->get(\App\Growth\Repository\ReferralAssistCountRepository::class)->add($assistCount);
        self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class)->flush();

        $superAdmin = $this->account('admin@practiceperfect.test');
        /** @var ReferralRuleService $referralRules */
        $referralRules = self::getContainer()->get(ReferralRuleService::class);
        $referralRules->update(5, 1, false, $superAdmin);

        $this->activateTenant($trainer);
        $reloaded = $this->assistCountFor($trainer, $referrer['player']);
        self::assertNotNull($reloaded);
        self::assertSame(2, $reloaded->getAssistCount(), 'AC-06-30: existing assist-count progress is preserved under the new ratio (example: 2 assists under an old rule keeps those 2 assists under a new one).');

        // AC-06-9/BR-06-4: the new rule applies platform-wide immediately —
        // re-reading it (from any context) reflects the change.
        self::assertSame(5, $referralRules->current()->referralsRequired);
    }

    /**
     * AC-06-31: every change is audit-logged — who, when, old value, new
     * value.
     */
    public function testRuleChangeIsAuditLogged(): void
    {
        $superAdmin = $this->account('admin@practiceperfect.test');
        /** @var ReferralRuleService $referralRules */
        $referralRules = self::getContainer()->get(ReferralRuleService::class);
        $referralRules->update(4, 3, true, $superAdmin);

        /** @var AuditLogEntryRepository $auditLog */
        $auditLog = self::getContainer()->get(AuditLogEntryRepository::class);
        $entries = $auditLog->findBy(['actionType' => 'platform_configuration.referral_rule_changed'], ['id' => 'DESC'], 1);

        self::assertNotEmpty($entries, 'AC-06-31: the change is audit-logged.');
        $entry = $entries[0];
        self::assertSame($superAdmin->getId(), $entry->getActorAccount()?->getId(), 'AC-06-31: who changed it.');
        self::assertArrayHasKey('before', $entry->getDetails());
        self::assertArrayHasKey('after', $entry->getDetails());
        self::assertSame(4, $entry->getDetails()['after']['referralsRequired']);
        self::assertSame(3, $entry->getDetails()['after']['tokensAwarded']);
    }

    /**
     * Only a Super Admin may edit the platform-wide rule.
     */
    public function testTrainerCannotAccessTheReferralRuleScreen(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/super-admin/growth/referral-rules');

        self::assertResponseStatusCodeSame(403);
    }
}
