<?php

declare(strict_types=1);

namespace App\Tests\Forms;

use App\Platform\Entity\FeatureToggle;
use App\Platform\Repository\FeatureToggleRepository;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\FormsFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * Epic-07's Camps feature toggle, wired to Epic-08's own routes — "The
 * trainer will lose the ability to create camp events"
 * (TrainerFeatureController::DISABLE_WARNINGS). Scoped to CREATION and the
 * public camp-submission surface only, never to managing/exporting
 * already-collected data, and never to Evaluations (BR-08-5: "always on";
 * the toggle is named "Camps," not "Forms") — see FormVoter's own docblock
 * for the full reasoning this test verifies.
 */
final class FormFeatureToggleTest extends WebTestCase
{
    use FixtureHelpers;
    use FormsFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    public function testDisabledCampsToggleBlocksNewCampCreation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->disableCamps($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/forms/camps/new');

        self::assertResponseStatusCodeSame(403);
    }

    public function testDisabledCampsToggleDoesNotBlockEvaluationCreation(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->disableCamps($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/forms/evaluations/new');

        self::assertResponseIsSuccessful();
    }

    /**
     * Scoped to creation and the public surface — an ALREADY-published
     * camp's trainer-side management (edit/view registrations) is not
     * blocked by disabling the toggle later. BR-07-3: "disabling never
     * deletes data."
     */
    public function testDisabledCampsToggleDoesNotBlockManagingAnExistingCamp(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Already Published Camp']);

        $this->disableCamps($trainer);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', sprintf('/trainer/forms/%d/edit', (int) $form->getId()));
        self::assertResponseIsSuccessful();

        $this->client->request('GET', sprintf('/trainer/forms/%d/submissions', (int) $form->getId()));
        self::assertResponseIsSuccessful();
    }

    /**
     * The public submission surface for a camp is also gated -- a trainer
     * should not be able to keep collecting camp registrations once Camps
     * is disabled platform-wide for them.
     */
    public function testDisabledCampsToggleClosesThePublicCampLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Gated Public Camp']);

        $this->disableCamps($trainer);

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Registration Closed');
    }

    /**
     * The same disabled toggle never touches an evaluation's own public
     * link -- BR-08-5's "always on" holds regardless of the Camps toggle.
     */
    public function testDisabledCampsToggleNeverClosesAnEvaluationLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createEvaluation($trainer, ['name' => 'Unaffected Evaluation']);

        $this->disableCamps($trainer);

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());

        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Registration Closed');
        self::assertSelectorTextContains('h1', 'Unaffected Evaluation');
    }

    /**
     * Re-enabling restores the create route and the public link
     * immediately (BR-07-3: "takes effect immediately").
     */
    public function testReenablingCampsRestoresCreationAndThePublicLink(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $form = $this->createCamp($trainer, ['name' => 'Restored Camp']);
        $this->disableCamps($trainer);

        /** @var FeatureToggleRepository $toggles */
        $toggles = self::getContainer()->get(FeatureToggleRepository::class);
        $toggle = $toggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_CAMPS);
        self::assertNotNull($toggle);
        $toggle->enable($this->account('admin@practiceperfect.test'));
        $this->formsEntityManagerFlush();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/forms/camps/new');
        self::assertResponseIsSuccessful();

        $this->client->request('GET', '/forms/'.$form->getShareableSlug());
        self::assertResponseIsSuccessful();
        self::assertSelectorTextNotContains('body', 'Registration Closed');
    }

    private function disableCamps(\App\Platform\Entity\Trainer $trainer): void
    {
        $this->activateTenant($trainer);
        /** @var FeatureToggleRepository $toggles */
        $toggles = self::getContainer()->get(FeatureToggleRepository::class);
        $toggles->seedDefaultsForTrainer($trainer, $this->account('admin@practiceperfect.test'));
        $this->formsEntityManagerFlush();

        $toggle = $toggles->findOneByTrainerAndFeature($trainer, FeatureToggle::FEATURE_CAMPS);
        self::assertNotNull($toggle);
        $toggle->disable($this->account('admin@practiceperfect.test'));
        $this->formsEntityManagerFlush();
    }

    private function formsEntityManagerFlush(): void
    {
        /** @var \Doctrine\ORM\EntityManagerInterface $entityManager */
        $entityManager = self::getContainer()->get(\Doctrine\ORM\EntityManagerInterface::class);
        $entityManager->flush();
    }
}
