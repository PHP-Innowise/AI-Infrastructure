<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Identity\Entity\Account;
use App\Identity\Repository\AccountRepository;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;

/**
 * Shared lookups for the fixture data every test in this suite starts from
 * (AppFixtures). Requires the including test case to expose
 * `self::getContainer()`, which every KernelTestCase/WebTestCase does.
 */
trait FixtureHelpers
{
    protected function account(string $email): Account
    {
        /** @var AccountRepository $repository */
        $repository = self::getContainer()->get(AccountRepository::class);
        $account = $repository->findOneByEmail($email);

        self::assertInstanceOf(Account::class, $account, sprintf('Fixture account "%s" is missing.', $email));

        return $account;
    }

    protected function trainer(string $slug): Trainer
    {
        /** @var TrainerRepository $repository */
        $repository = self::getContainer()->get(TrainerRepository::class);
        $trainer = $repository->findOneBySlug($slug);

        self::assertInstanceOf(Trainer::class, $trainer, sprintf('Fixture trainer "%s" is missing.', $slug));

        return $trainer;
    }

    protected function activateTenant(Trainer $trainer): void
    {
        /** @var TenantContext $tenantContext */
        $tenantContext = self::getContainer()->get(TenantContext::class);
        $tenantContext->activateFor($trainer);
    }
}
