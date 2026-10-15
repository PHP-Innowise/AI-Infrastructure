<?php

declare(strict_types=1);

namespace App\Tests\Crm;

use App\Crm\Repository\LabelRepository;
use App\Crm\Repository\PlayerLabelRepository;
use App\Identity\Repository\PlayerProfileRepository;
use App\Tests\Support\CrmFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use App\Tests\Support\SchedulingFixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-03.03 — Trainer Creates and Applies Labels.
 */
final class LabelManagementTest extends WebTestCase
{
    use FixtureHelpers;
    use SchedulingFixtureHelpers;
    use CrmFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-03-11: name (max 50 chars) and a preset color; saving creates the
     * label and it appears in the list.
     */
    public function testTrainerCreatesALabel(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $name = 'Elite Squad '.uniqid();

        $crawler = $this->client->request('GET', '/trainer/labels/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save label')->form([
            'label[name]' => $name,
            'label[colorHex]' => '#00B300',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/labels');
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', $name);
    }

    /**
     * AC-03-12: multi-select apply from the player detail view; applied
     * labels show as colored badges and a player can have multiple.
     */
    public function testTrainerAppliesLabelsToAPlayer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $nameA = 'Elite Squad '.uniqid();
        $nameB = 'Scholarship Track '.uniqid();
        $labelA = $this->createLabel($trainer, $nameA, '#00B300');
        $labelB = $this->createLabel($trainer, $nameB, '#0066CC');
        $membership = $this->membershipFor($trainer, $this->patPlayer());

        $trainerAccount = $this->account('trainer@practiceperfect.test');
        $this->client->loginUser($trainerAccount);

        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        self::assertResponseIsSuccessful();

        // Expanded, multiple-choice EntityType checkboxes don't lend
        // themselves to Form::submit()'s single-field API cleanly — posting
        // the array directly (with the real CSRF token from the rendered
        // page) is the more robust way to exercise this specific shape.
        $token = $crawler->filter('form[action*="/labels"] input[name="apply_labels[_token]"]')->attr('value');
        $this->client->request('POST', '/trainer/players/'.$membership->getId().'/labels', [
            'apply_labels' => [
                'labels' => [(string) $labelA->getId(), (string) $labelB->getId()],
                '_token' => $token,
            ],
        ]);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', $nameA);
        self::assertSelectorTextContains('body', $nameB);

        $this->activateTenant($trainer);
        /** @var PlayerLabelRepository $playerLabels */
        $playerLabels = self::getContainer()->get(PlayerLabelRepository::class);
        $applied = $playerLabels->findForPlayer($this->patPlayer());
        $appliedForThisTest = array_filter($applied, static fn ($pl) => \in_array($pl->getLabel()->getId(), [$labelA->getId(), $labelB->getId()], true));
        self::assertCount(2, $appliedForThisTest, 'AC-03-12: a player can have multiple labels.');
    }

    /**
     * AC-03-28: the player detail view's Labels section supports
     * click-to-remove — removing a player's own association with a label,
     * never the label itself (BR-03-4).
     */
    public function testTrainerRemovesALabelFromOnePlayerViaClickToRemove(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $membership = $this->freshPlayerMembership($trainer);
        $label = $this->createLabel($trainer, 'ClickRemove'.uniqid(), '#00B300');
        $this->applyLabelToPlayer($trainer, $membership->getPlayer(), $label, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/players/'.$membership->getId());
        self::assertSelectorTextContains('body', $label->getName());

        // The remove button's visible text is a bare "×" (its accessible
        // name lives in aria-label, which Crawler::selectButton() does not
        // match against) — select the form by its action URL instead.
        $formNode = $crawler->filter('form[action*="/labels/'.$label->getId().'/remove"]');
        self::assertGreaterThan(0, $formNode->count());
        $form = $formNode->form();
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->activateTenant($trainer);
        /** @var LabelRepository $labels */
        $labels = self::getContainer()->get(LabelRepository::class);
        self::assertNotNull($labels->find($label->getId()), 'BR-03-4: removing from the player does not delete the label itself.');
        /** @var PlayerLabelRepository $playerLabels */
        $playerLabels = self::getContainer()->get(PlayerLabelRepository::class);
        self::assertEmpty($playerLabels->findForPlayer($membership->getPlayer()), 'AC-03-28: click-to-remove.');
    }

    /**
     * AC-03-13: edit a label's name/color.
     */
    public function testTrainerEditsALabel(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $label = $this->createLabel($trainer, 'Old Name '.uniqid(), '#00B300');
        $newName = 'New Name '.uniqid();

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/labels/'.$label->getId().'/edit');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save label')->form([
            'label[name]' => $newName,
            'label[colorHex]' => '#CC0000',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/trainer/labels');

        $this->activateTenant($trainer);
        /** @var LabelRepository $labels */
        $labels = self::getContainer()->get(LabelRepository::class);
        $reloaded = $labels->find($label->getId());
        self::assertNotNull($reloaded);
        self::assertSame($newName, $reloaded->getName());
        self::assertSame('#CC0000', $reloaded->getColorHex());
    }

    /**
     * AC-03-13/BR-03-5: deleting a label removes it from every player it was
     * applied to, with a confirmation naming the affected player count —
     * and does not delete the player itself.
     */
    public function testDeletingALabelRemovesItFromEveryPlayerButKeepsThePlayers(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $label = $this->createLabel($trainer, 'Temp Label '.uniqid(), '#00B300');
        $trainerAccount = $trainer->getOwnerAccount();
        $this->applyLabelToPlayer($trainer, $this->patPlayer(), $label, $trainerAccount);
        $this->applyLabelToPlayer($trainer, $this->alexPlayer(), $label, $trainerAccount);

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/labels/'.$label->getId().'/delete');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', '2'); // BR-03-5: "...from [N] players?"

        $form = $crawler->selectButton('Confirm delete')->form();
        $this->client->submit($form);
        self::assertResponseRedirects('/trainer/labels');

        $this->activateTenant($trainer);
        /** @var LabelRepository $labels */
        $labels = self::getContainer()->get(LabelRepository::class);
        self::assertNull($labels->find($label->getId()), 'BR-03-5: the label itself is gone.');

        /** @var PlayerProfileRepository $players */
        $players = self::getContainer()->get(PlayerProfileRepository::class);
        self::assertNotNull($players->find($this->patPlayer()->getId()), 'BR-03-4: removing a label does not delete the player.');
    }

    /**
     * AC-03-14: each label shows "Applied to N players."
     */
    public function testLabelShowsItsUsageCount(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $label = $this->createLabel($trainer, 'Counted Label '.uniqid(), '#00B300');
        $this->applyLabelToPlayer($trainer, $this->patPlayer(), $label, $trainer->getOwnerAccount());
        $this->applyLabelToPlayer($trainer, $this->alexPlayer(), $label, $trainer->getOwnerAccount());

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $this->client->request('GET', '/trainer/labels');

        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Applied to 2 players');
    }

    /**
     * BR-03-3: label names collide case-insensitively within one trainer.
     */
    public function testLabelNamesCollideCaseInsensitivelyWithinOneTrainer(): void
    {
        $trainer = $this->trainer('peak-performance');
        $this->activateTenant($trainer);
        $base = 'CaseTest'.uniqid();
        $this->createLabel($trainer, $base, '#00B300');

        $this->client->loginUser($this->account('trainer@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/trainer/labels/new');
        $form = $crawler->selectButton('Save label')->form([
            'label[name]' => strtolower($base),
            'label[colorHex]' => '#CC0000',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422);
        self::assertSelectorTextContains('body', 'already exists');
    }

    /**
     * BR-03-4: the same label name created by two different trainers are
     * distinct labels — no cross-tenant collision.
     */
    public function testTheSameLabelNameAcrossTwoTrainersAreDistinctLabels(): void
    {
        $sharedName = 'CrossTenant'.uniqid();

        $trainerA = $this->trainer('peak-performance');
        $this->activateTenant($trainerA);
        $this->createLabel($trainerA, $sharedName, '#00B300');

        $trainerB = $this->trainer('baseline-athletics');
        $this->activateTenant($trainerB);
        $labelB = $this->createLabel($trainerB, $sharedName, '#0066CC');

        self::assertNotNull($labelB->getId(), 'BR-03-4: a second trainer can create the same-named label without collision.');
    }
}
