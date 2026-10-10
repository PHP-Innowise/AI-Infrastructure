<?php

declare(strict_types=1);

namespace App\Tests\Content;

use App\Content\Entity\Drill;
use App\Content\Repository\DrillRepository;
use App\Tests\Support\ContentFixtureHelpers;
use App\Tests\Support\FixtureHelpers;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-04.02 — Trainer Creates Drill in Practice Pillar.
 */
final class DrillCreationTest extends WebTestCase
{
    use FixtureHelpers;
    use ContentFixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * AC-04-4: required name (max 100), optional instructions (max 1000),
     * required YouTube URL, difficulty, equipment, space, player count,
     * duration range, one or more categories, Private/Public toggle.
     * AC-04-5: saving adds it to "My Drills" with a confirmation.
     */
    public function testTrainerCreatesDrillWithRequiredAndOptionalFields(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/drills/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save drill')->form([
            'drill[title]' => 'Cone Weave Sprint',
            'drill[youtubeUrl]' => 'https://www.youtube.com/watch?v=abc12345678',
            'drill[instructions]' => 'Set up 6 cones in a line. Weave through at full speed.',
            'drill[difficultyLevel]' => Drill::DIFFICULTY_INTERMEDIATE,
            'drill[categories]' => 'Dribbling, Footwork',
            'drill[equipment]' => 'Cones, Ball',
            'drill[spaceRequirement]' => Drill::SPACE_MEDIUM,
            'drill[playerCount]' => '1-2',
            'drill[durationMinMinutes]' => '5',
            'drill[durationMaxMinutes]' => '10',
            'drill[isPublic]' => '0',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects();
        $this->client->followRedirect();
        self::assertSelectorTextContains('body', 'Drill created');

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var DrillRepository $drills */
        $drills = self::getContainer()->get(DrillRepository::class);
        $created = current(array_filter(
            $drills->findOwnForActiveTenant($this->trainer('peak-performance')),
            static fn (Drill $d): bool => 'Cone Weave Sprint' === $d->getTitle(),
        ));

        self::assertNotFalse($created, 'AC-04-5: the drill appears in "My Drills".');
        self::assertSame(Drill::DIFFICULTY_INTERMEDIATE, $created->getDifficultyLevel());
        self::assertSame(['Dribbling', 'Footwork'], $created->getCategories());
        self::assertSame(['Cones', 'Ball'], $created->getEquipment());
        self::assertSame(Drill::SPACE_MEDIUM, $created->getSpaceRequirement());
        self::assertSame('1-2', $created->getPlayerCount());
        self::assertSame(5, $created->getDurationMinMinutes());
        self::assertSame(10, $created->getDurationMaxMinutes());
        self::assertFalse($created->isPublic());
    }

    /**
     * AC-04-5: marking Public at creation makes it immediately discoverable.
     */
    public function testDrillMarkedPublicAtCreationAppearsInPublicDiscovery(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/drills/new');
        $form = $crawler->selectButton('Save drill')->form([
            'drill[title]' => 'Public From The Start',
            'drill[youtubeUrl]' => 'https://www.youtube.com/watch?v=xyz98765432',
            'drill[difficultyLevel]' => Drill::DIFFICULTY_BEGINNER,
            'drill[categories]' => 'Shooting',
            'drill[isPublic]' => '1',
        ]);
        $this->client->submit($form);
        self::assertResponseRedirects();

        $this->client->loginUser($this->account('trainer-b@practiceperfect.test'));
        $this->client->request('GET', '/trainer/content/drills?scope=public');
        self::assertResponseIsSuccessful();
        self::assertSelectorTextContains('body', 'Public From The Start');
    }

    /**
     * AC-04-6, "Validation": a name is required.
     */
    public function testDrillNameIsRequired(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/drills/new');
        $form = $crawler->selectButton('Save drill')->form([
            'drill[title]' => '',
            'drill[youtubeUrl]' => 'https://www.youtube.com/watch?v=abc12345678',
            'drill[difficultyLevel]' => Drill::DIFFICULTY_BEGINNER,
            'drill[categories]' => 'Shooting',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
    }

    /**
     * AC-04-6, "Validation": a valid-format YouTube URL is required.
     */
    public function testDrillRequiresValidYoutubeUrlFormat(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/drills/new');
        $form = $crawler->selectButton('Save drill')->form([
            'drill[title]' => 'Bad URL Drill',
            'drill[youtubeUrl]' => 'not-a-url-at-all',
            'drill[difficultyLevel]' => Drill::DIFFICULTY_BEGINNER,
            'drill[categories]' => 'Shooting',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();
    }

    /**
     * AC-04-6, "Validation": at least one category is required.
     */
    public function testDrillRequiresAtLeastOneCategory(): void
    {
        $this->client->loginUser($this->account('trainer@practiceperfect.test'));

        $crawler = $this->client->request('GET', '/trainer/content/drills/new');
        $form = $crawler->selectButton('Save drill')->form([
            'drill[title]' => 'No Category Drill',
            'drill[youtubeUrl]' => 'https://www.youtube.com/watch?v=abc12345678',
            'drill[difficultyLevel]' => Drill::DIFFICULTY_BEGINNER,
            'drill[categories]' => '',
        ]);
        $this->client->submit($form);

        self::assertResponseIsUnprocessable();

        $this->activateTenant($this->trainer('peak-performance'));
        /** @var DrillRepository $drills */
        $drills = self::getContainer()->get(DrillRepository::class);
        self::assertEmpty(array_filter(
            $drills->findOwnForActiveTenant($this->trainer('peak-performance')),
            static fn (Drill $d): bool => 'No Category Drill' === $d->getTitle(),
        ), 'The rejected drill was never persisted.');
    }
}
