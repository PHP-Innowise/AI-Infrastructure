<?php

declare(strict_types=1);

namespace App\Tests\Platform;

use App\Identity\Entity\PlayerTrainerMembership;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;
use App\Platform\Tenancy\TenantNotResolvedException;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * Tenancy isolation is the highest-risk defect class in this product: a leak
 * would disclose minors' medical and financial-aid flags to the wrong trainer.
 * These tests exercise the two layers that fail loudly and fail closed.
 *
 * @see specs/architect-architecture.md "Tenancy enforcement"
 */
final class TenancyIsolationTest extends KernelTestCase
{
    private EntityManagerInterface $entityManager;
    private TenantContext $tenantContext;

    protected function setUp(): void
    {
        self::bootKernel();

        $this->entityManager = self::getContainer()->get(EntityManagerInterface::class);
        $this->tenantContext = self::getContainer()->get(TenantContext::class);
    }

    protected function tearDown(): void
    {
        $this->tenantContext->clear();

        parent::tearDown();
    }

    /**
     * AC-01-74: trainers see only their own organization's players (the
     * player-trainer relationship, which is what `PlayerTrainerMembership`
     * is); the coach side of the same criterion is exercised at the HTTP
     * layer by CoachListTenancyTest. The central guarantee: the same query,
     * run under two tenants, returns each tenant's own rows and never the
     * other's.
     */
    public function testATrainerCannotSeeAnotherTrainersMemberships(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        $peakRows = $this->allMemberships();

        $this->entityManager->clear();

        $this->tenantContext->activateFor($baseline);
        $baselineRows = $this->allMemberships();

        self::assertNotEmpty($peakRows, 'Fixtures should give Peak Performance at least one membership.');
        self::assertNotEmpty($baselineRows, 'Fixtures should give Baseline Athletics at least one membership.');

        foreach ($peakRows as $row) {
            self::assertSame($peak->getId(), $row->getTrainer()->getId());
        }

        foreach ($baselineRows as $row) {
            self::assertSame($baseline->getId(), $row->getTrainer()->getId());
        }

        $peakIds = array_map(static fn (PlayerTrainerMembership $m): ?int => $m->getId(), $peakRows);
        $baselineIds = array_map(static fn (PlayerTrainerMembership $m): ?int => $m->getId(), $baselineRows);

        self::assertSame([], array_intersect($peakIds, $baselineIds), 'The two tenants returned overlapping rows.');
    }

    /**
     * Fail closed. With no tenant established, a trainer-scoped read must
     * yield nothing — never the whole table.
     */
    public function testAnUnresolvedTenantSeesNoTrainerScopedRowsAtAll(): void
    {
        $this->tenantContext->clear();

        self::assertSame([], $this->allMemberships());
    }

    /**
     * Fail loud. RLS returning zero rows reads exactly like "there is no
     * data", which is how an unscoped batch job processes nothing forever and
     * nobody notices. TenantContext turns that silence into an exception.
     */
    public function testRequiringATenantThrowsWhenNoneIsResolved(): void
    {
        $this->tenantContext->clear();

        $this->expectException(TenantNotResolvedException::class);

        $this->tenantContext->requireTrainerId();
    }

    /**
     * The WITH CHECK half of the policy. Reading is not the only direction
     * that leaks: without it, a caller could insert into another trainer's
     * tenant and simply never read it back.
     */
    public function testWritingIntoAnotherTenantIsRejected(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        // Tenant is Peak Performance, but the row claims Baseline Athletics.
        $this->tenantContext->activateFor($peak);

        $player = $this->entityManager
            ->getRepository(\App\Identity\Entity\PlayerProfile::class)
            ->findOneBy([]);

        self::assertNotNull($player, 'Fixtures should provide at least one player profile.');

        $this->entityManager->persist(
            new PlayerTrainerMembership($baseline, $player, PlayerTrainerMembership::SOURCE_SHARELINK)
        );

        $this->expectException(\Doctrine\DBAL\Exception::class);
        $this->entityManager->flush();
    }

    /**
     * @return list<PlayerTrainerMembership>
     */
    private function allMemberships(): array
    {
        /** @var list<PlayerTrainerMembership> $rows */
        $rows = $this->entityManager
            ->createQuery('SELECT m FROM '.PlayerTrainerMembership::class.' m')
            ->getResult();

        return $rows;
    }

    private function trainer(string $slug): Trainer
    {
        /** @var TrainerRepository $repository */
        $repository = self::getContainer()->get(TrainerRepository::class);
        $trainer = $repository->findOneBySlug($slug);

        self::assertInstanceOf(Trainer::class, $trainer, sprintf('Fixture trainer "%s" is missing.', $slug));

        return $trainer;
    }
}
