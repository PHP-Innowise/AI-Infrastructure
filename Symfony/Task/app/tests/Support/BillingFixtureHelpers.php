<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Billing\Entity\TokenPackage;
use App\Billing\Entity\TrainerBillingSettings;
use App\Billing\Repository\TokenPackageRepository;
use App\Billing\Repository\TrainerBillingSettingsRepository;
use App\Billing\Service\TokenLedgerService;
use App\Billing\Stripe\InMemoryStripeClient;
use App\Billing\Stripe\StripeClient;
use App\Identity\Entity\Account;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Billing-specific (Epic-05) fixture helpers, built on FixtureHelpers (the
 * base AppFixtures lookups, tenant activation) — the same "set up via the
 * entity/service layer, exercise the feature under test via HTTP" split
 * SchedulingFixtureHelpers/ContentFixtureHelpers already establish.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`. The tenant must already be active
 * (`activateTenant($trainer)`) before calling any of these.
 */
trait BillingFixtureHelpers
{
    /**
     * AC-05-1: marks $trainer as having completed Stripe Connect
     * onboarding — there is no real Stripe redirect to drive this through
     * in a test, so it goes directly through `InMemoryStripeClient`'s own
     * test seam, exactly as a real `account.updated` webhook would leave
     * the row.
     */
    protected function connectStripe(Trainer $trainer): void
    {
        /** @var TrainerBillingSettingsRepository $settingsRepo */
        $settingsRepo = self::getContainer()->get(TrainerBillingSettingsRepository::class);
        $settings = $settingsRepo->getOrCreateForTrainer($trainer);

        $accountId = 'acct_fixture_'.$trainer->getId();
        $settings->attachStripeConnectAccount($accountId);
        $settings->updateOnboardingStatus(TrainerBillingSettings::ONBOARDING_COMPLETE);
        $this->billingEntityManager()->flush();

        $this->fakeStripeClient()->setAccountOnboardingComplete($accountId);
    }

    /**
     * @param array{tokenCount?: int, priceMinorUnits?: int, isActive?: bool} $overrides
     */
    protected function createTokenPackage(Trainer $trainer, string $label, array $overrides = []): TokenPackage
    {
        $package = new TokenPackage(
            $trainer,
            $label,
            $overrides['tokenCount'] ?? 10,
            $overrides['priceMinorUnits'] ?? 9000,
            $overrides['isActive'] ?? true,
        );

        /** @var TokenPackageRepository $packages */
        $packages = self::getContainer()->get(TokenPackageRepository::class);
        $packages->add($package);
        $this->billingEntityManager()->flush();

        return $package;
    }

    /**
     * Grants tokens directly through TokenLedgerService::gift() (never by
     * setting TokenBalance's own field), so fixture setup obeys I1 (the
     * projection is always the sum of real entries) exactly like a real
     * gift would.
     */
    protected function giveTokens(Trainer $trainer, Account $parentAccount, int $amount, ?Account $grantedBy = null): void
    {
        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);
        $ledger->gift($trainer, $parentAccount, $amount, $grantedBy ?? $trainer->getOwnerAccount(), 'Test fixture grant.');
    }

    protected function tokenBalance(Trainer $trainer, Account $parentAccount): int
    {
        /** @var TokenLedgerService $ledger */
        $ledger = self::getContainer()->get(TokenLedgerService::class);

        return $ledger->balanceFor($trainer, $parentAccount);
    }

    protected function fakeStripeClient(): InMemoryStripeClient
    {
        /** @var StripeClient $client */
        $client = self::getContainer()->get(StripeClient::class);
        self::assertInstanceOf(InMemoryStripeClient::class, $client, 'The test container must bind StripeClient to InMemoryStripeClient.');

        return $client;
    }

    private function billingEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
