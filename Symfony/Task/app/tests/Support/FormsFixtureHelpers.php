<?php

declare(strict_types=1);

namespace App\Tests\Support;

use App\Forms\Dto\FormField;
use App\Forms\Entity\Form;
use App\Forms\Repository\FormRepository;
use App\Forms\Service\FormService;
use App\Platform\Entity\Trainer;
use Doctrine\ORM\EntityManagerInterface;

/**
 * Forms-specific (Epic-08) fixture helpers, built on FixtureHelpers
 * (AppFixtures lookups, tenant activation) — the same "set up via the
 * entity/service layer, exercise the feature under test via HTTP" split
 * BillingFixtureHelpers/ContentFixtureHelpers already establish.
 *
 * Requires the including test case to also `use FixtureHelpers` and expose
 * `self::getContainer()`. The tenant must already be active
 * (`activateTenant($trainer)`) before calling any of these.
 */
trait FormsFixtureHelpers
{
    /**
     * @param array{name?: string, description?: ?string, capacityLimit?: int, priceMinorUnits?: ?int, fields?: list<FormField>, publish?: bool} $overrides
     */
    protected function createCamp(Trainer $trainer, array $overrides = []): Form
    {
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);

        $form = $formService->createCamp(
            $trainer,
            $overrides['name'] ?? 'Test Camp '.uniqid(),
            $overrides['description'] ?? 'A test camp.',
            \array_key_exists('priceMinorUnits', $overrides) ? $overrides['priceMinorUnits'] : null,
            $overrides['capacityLimit'] ?? 10,
            $overrides['fields'] ?? self::defaultTestFields(),
        );

        if ($overrides['publish'] ?? true) {
            $formService->publish($form);
        }

        return $form;
    }

    /**
     * @param array{name?: string, description?: ?string, priceMinorUnits?: ?int, fields?: list<FormField>, publish?: bool} $overrides
     */
    protected function createEvaluation(Trainer $trainer, array $overrides = []): Form
    {
        /** @var FormService $formService */
        $formService = self::getContainer()->get(FormService::class);

        $form = $formService->createEvaluation(
            $trainer,
            $overrides['name'] ?? 'Test Evaluation '.uniqid(),
            $overrides['description'] ?? 'A test evaluation.',
            \array_key_exists('priceMinorUnits', $overrides) ? $overrides['priceMinorUnits'] : null,
            $overrides['fields'] ?? self::defaultTestFields(),
        );

        if ($overrides['publish'] ?? true) {
            $formService->publish($form);
        }

        return $form;
    }

    /**
     * @return list<FormField>
     */
    protected static function defaultTestFields(): array
    {
        return FormService::defaultTemplateFields();
    }

    protected function findForm(string $shareableSlug): Form
    {
        /** @var FormRepository $forms */
        $forms = self::getContainer()->get(FormRepository::class);
        $form = $forms->findOneBySlug($shareableSlug);

        self::assertInstanceOf(Form::class, $form, sprintf('Form with slug "%s" not found.', $shareableSlug));

        return $form;
    }

    private function formsEntityManager(): EntityManagerInterface
    {
        /** @var EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(EntityManagerInterface::class);

        return $entityManager;
    }
}
