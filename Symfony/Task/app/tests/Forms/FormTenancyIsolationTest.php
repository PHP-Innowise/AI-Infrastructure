<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Forms\Entity\Form;
use App\Forms\Entity\FormSubmission;
use App\Forms\Service\FormService;
use App\Platform\Entity\Trainer;
use App\Platform\Repository\TrainerRepository;
use App\Platform\Tenancy\TenantContext;
use Doctrine\ORM\EntityManagerInterface;
use Symfony\Bundle\FrameworkBundle\Test\KernelTestCase;

/**
 * `form` and `form_submission` are this epic's own two additions to the
 * trainer-scoped tenancy manifest, and `form_submission` is singled out by
 * the schema doc as "the only public, unauthenticated write in the
 * platform" — the highest-risk defect class in this product
 * (TenancyIsolationTest's own docblock), now reachable from a bearer code
 * rather than only from an authenticated session. These tests mirror that
 * class's own structure for the two new tables.
 *
 * @see specs/architect-architecture.md "Tenancy enforcement"
 * @see specs/database-designer-schema.md "`form_submission`"
 */
final class FormTenancyIsolationTest extends KernelTestCase
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
     * The same query, run under two tenants, returns each tenant's own
     * forms and never the other's.
     */
    public function testATrainerCannotSeeAnotherTrainersForms(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);
        $peakForm = $formService->createCamp($peak, 'Peak Isolation Camp', null, null, 10, FormService::defaultTemplateFields());
        $baselineForm = null;

        $this->tenantContext->activateFor($baseline);
        $baselineForm = $formService->createCamp($baseline, 'Baseline Isolation Camp', null, null, 10, FormService::defaultTemplateFields());

        $this->entityManager->clear();

        $this->tenantContext->activateFor($peak);
        $peakRows = $this->allForms();

        $this->entityManager->clear();

        $this->tenantContext->activateFor($baseline);
        $baselineRows = $this->allForms();

        $peakIds = array_map(static fn (Form $f): ?int => $f->getId(), $peakRows);
        $baselineIds = array_map(static fn (Form $f): ?int => $f->getId(), $baselineRows);

        self::assertContains($peakForm->getId(), $peakIds);
        self::assertContains($baselineForm->getId(), $baselineIds);
        self::assertSame([], array_intersect($peakIds, $baselineIds), 'The two tenants returned overlapping form rows.');
    }

    /**
     * Same guarantee for `form_submission` — the table the schema doc
     * itself flags as the platform's only unauthenticated write.
     */
    public function testATrainerCannotSeeAnotherTrainersFormSubmissions(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);
        $peakForm = $formService->createCamp($peak, 'Peak Submission Camp', null, null, 10, FormService::defaultTemplateFields());
        $peakSubmission = new FormSubmission(
            $peak,
            $peakForm,
            ['participant_name' => 'Peak Kid', 'participant_email' => 'peak-kid@example.test'],
            'peak-kid@example.test',
            FormSubmission::STATUS_FREE,
        );
        $this->entityManager->persist($peakSubmission);
        $this->entityManager->flush();

        $this->tenantContext->activateFor($baseline);
        $baselineForm = $formService->createCamp($baseline, 'Baseline Submission Camp', null, null, 10, FormService::defaultTemplateFields());
        $baselineSubmission = new FormSubmission(
            $baseline,
            $baselineForm,
            ['participant_name' => 'Baseline Kid', 'participant_email' => 'baseline-kid@example.test'],
            'baseline-kid@example.test',
            FormSubmission::STATUS_FREE,
        );
        $this->entityManager->persist($baselineSubmission);
        $this->entityManager->flush();

        $this->entityManager->clear();

        $this->tenantContext->activateFor($peak);
        $peakRows = $this->allSubmissions();
        self::assertNotEmpty($peakRows);
        foreach ($peakRows as $row) {
            self::assertSame($peak->getId(), $row->getTrainer()->getId());
            self::assertNotSame('baseline-kid@example.test', $row->getContactEmail(), "Peak Performance must never see Baseline's participant PII.");
        }

        $this->entityManager->clear();

        $this->tenantContext->activateFor($baseline);
        $baselineRows = $this->allSubmissions();
        self::assertNotEmpty($baselineRows);
        foreach ($baselineRows as $row) {
            self::assertSame($baseline->getId(), $row->getTrainer()->getId());
            self::assertNotSame('peak-kid@example.test', $row->getContactEmail(), "Baseline Athletics must never see Peak's participant PII.");
        }
    }

    /**
     * Fail closed: with no tenant established, reading `form` or
     * `form_submission` yields nothing — never the whole table.
     */
    public function testAnUnresolvedTenantSeesNoFormsOrSubmissions(): void
    {
        $this->tenantContext->clear();

        self::assertSame([], $this->allForms());
        self::assertSame([], $this->allSubmissions());
    }

    /**
     * The WITH CHECK half: a `form_submission` row that claims a different
     * trainer than the active tenant is rejected at the database level,
     * not merely filtered on read. This is exactly the write path a
     * misconfigured or spoofed public-code resolution would take.
     */
    public function testWritingAFormSubmissionIntoAnotherTenantIsRejected(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);
        $peakForm = $formService->createCamp($peak, 'RLS Check Camp', null, null, 10, FormService::defaultTemplateFields());

        // Tenant is Peak Performance, but the row claims Baseline Athletics.
        $this->entityManager->persist(new FormSubmission(
            $baseline,
            $peakForm,
            ['participant_name' => 'Spoofed', 'participant_email' => 'spoofed@example.test'],
            'spoofed@example.test',
            FormSubmission::STATUS_FREE,
        ));

        $this->expectException(\Doctrine\DBAL\Exception::class);
        $this->entityManager->flush();
    }

    /**
     * The resolver's own boundary: a public code minted for one trainer
     * never resolves to a different one, regardless of which tenant (if
     * any) happens to already be active when the lookup runs — matching
     * "the tenant is derived from the code, never accepted from context"
     * (council-sharelink-tenant-resolution.md).
     */
    public function testPublicCodeResolvesOnlyItsOwnTrainerRegardlessOfAmbientTenant(): void
    {
        $peak = $this->trainer('peak-performance');
        $baseline = $this->trainer('baseline-athletics');

        $this->tenantContext->activateFor($peak);
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);
        $peakForm = $formService->createCamp($peak, 'Resolver Boundary Camp', null, null, 10, FormService::defaultTemplateFields());
        $formService->publish($peakForm);

        // Switch the ambient tenant to Baseline -- the code must still
        // resolve to Peak, never silently adopt the ambient tenant.
        $this->tenantContext->activateFor($baseline);

        /** @var \App\Platform\Repository\PublicTenantCodeRepository $codes */
        $codes = self::getContainer()->get(\App\Platform\Repository\PublicTenantCodeRepository::class);
        $resolvedTrainerId = $codes->findTrainerIdByCode($peakForm->getShareableSlug());

        self::assertSame($peak->getId(), $resolvedTrainerId);
        self::assertNotSame($baseline->getId(), $resolvedTrainerId);
    }

    /**
     * @return list<Form>
     */
    private function allForms(): array
    {
        /** @var list<Form> $rows */
        $rows = $this->entityManager->createQuery('SELECT f FROM '.Form::class.' f')->getResult();

        return $rows;
    }

    /**
     * @return list<FormSubmission>
     */
    private function allSubmissions(): array
    {
        /** @var list<FormSubmission> $rows */
        $rows = $this->entityManager->createQuery('SELECT s FROM '.FormSubmission::class.' s')->getResult();

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
