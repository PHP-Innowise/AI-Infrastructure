<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\PlayerProfile;
use App\Identity\Repository\PlayerProfileRepository;
use App\Identity\Service\ChildProfileService;
use App\Tests\Support\FixtureHelpers;
use PHPUnit\Framework\Attributes\DataProvider;
use Symfony\Bundle\FrameworkBundle\KernelBrowser;
use Symfony\Bundle\FrameworkBundle\Test\WebTestCase;

/**
 * US-01.03 § Validation — "Age: 1-18 years (children only, adults use own
 * accounts)".
 *
 * The rule was implemented, in `ChildProfileService`, and a child entered as
 * born in 2030 was accepted anyway and displayed as "Age 3". Not because the
 * range check was missing: because it asked `DateInterval::$y`, which carries
 * no sign, so a date four years in the future came back as a small positive
 * age that sat comfortably inside 1-18.
 *
 * One unsigned subtraction, two symptoms — a bypassed validation rule and a
 * plausible-looking wrong number on screen. These tests pin both, and the
 * boundaries the rule actually has.
 */
final class ChildAgeValidationTest extends WebTestCase
{
    use FixtureHelpers;

    private KernelBrowser $client;

    protected function setUp(): void
    {
        $this->client = self::createClient();
    }

    /**
     * The calculation, on its own, in both directions.
     */
    public function testAgeIsSignedSoAFutureDateOfBirthIsNegative(): void
    {
        $on = new \DateTimeImmutable('2026-08-12');

        self::assertSame(14, PlayerProfile::ageInYears(new \DateTimeImmutable('2012-04-18'), $on));
        self::assertSame(0, PlayerProfile::ageInYears(new \DateTimeImmutable('2026-01-01'), $on));
        self::assertSame(
            -3,
            PlayerProfile::ageInYears(new \DateTimeImmutable('2030-01-01'), $on),
            'An unborn child is not three years old — the reading that let one through.',
        );
    }

    /**
     * Through the service, which is where the range is an invariant rather
     * than a form's opinion.
     *
     * @param non-empty-string $dateOfBirth
     */
    #[DataProvider('rejectedDatesProvider')]
    public function testTheServiceRefusesAnAgeOutsideOneToEighteen(string $dateOfBirth, string $why): void
    {
        self::bootKernel();
        $parent = $this->account('player@practiceperfect.test');

        /** @var ChildProfileService $children */
        $children = self::getContainer()->get(ChildProfileService::class);

        $this->expectException(\InvalidArgumentException::class);
        $children->createChild($parent, 'Out Of Range', new \DateTimeImmutable($dateOfBirth), null, null);
        self::fail($why);
    }

    /**
     * @return iterable<string, array{string, string}>
     */
    public static function rejectedDatesProvider(): iterable
    {
        yield 'born in the future' => ['+4 years', 'A child not yet born was accepted.'];
        yield 'born today' => ['today', 'A newborn is under the stated minimum of 1.'];
        yield 'an adult' => ['-40 years', 'US-01.03: adults use their own accounts.'];
        yield 'exactly nineteen' => ['-19 years', 'Someone born exactly 19 years ago is 19 today.'];
    }

    /**
     * The boundaries the rule includes, so the fix does not overshoot into
     * refusing legitimate children.
     *
     * @param non-empty-string $dateOfBirth
     */
    #[DataProvider('acceptedDatesProvider')]
    public function testTheServiceAcceptsTheEdgesOfTheRange(string $dateOfBirth): void
    {
        self::bootKernel();
        $parent = $this->account('player@practiceperfect.test');

        /** @var ChildProfileService $children */
        $children = self::getContainer()->get(ChildProfileService::class);
        $child = $children->createChild($parent, 'In Range '.uniqid(), new \DateTimeImmutable($dateOfBirth), null, null);

        self::assertNotNull($child->getId());
    }

    /**
     * @return iterable<string, array{string}>
     */
    public static function acceptedDatesProvider(): iterable
    {
        yield 'just turned one' => ['-1 year'];
        yield 'mid range' => ['-12 years'];
        yield 'still eighteen' => ['-19 years +1 day'];
    }

    /**
     * Over HTTP, where a parent actually meets the rule: the message belongs
     * on the date field they typed, not at the top of the page.
     */
    public function testTheAddChildFormRejectsAFutureDateOfBirthOnTheField(): void
    {
        $this->client->loginUser($this->account('player@practiceperfect.test'));
        $crawler = $this->client->request('GET', '/portal/family/children/new');
        self::assertResponseIsSuccessful();

        $form = $crawler->selectButton('Save child profile')->form([
            'child_profile[firstName]' => 'Not Yet Born',
            'child_profile[dateOfBirth]' => (new \DateTimeImmutable('+4 years'))->format('Y-m-d'),
        ]);
        $this->client->submit($form);

        self::assertSelectorExists(
            '#child_profile_dateOfBirth[aria-invalid="true"]',
            'The rejection must be attached to the date of birth.',
        );

        /** @var PlayerProfileRepository $profiles */
        $profiles = self::getContainer()->get(PlayerProfileRepository::class);
        self::assertNull($profiles->findOneBy(['firstName' => 'Not Yet Born']), 'Nothing is created for an impossible date.');
    }

    /**
     * US-01.03 again, from the other side: "the parent account is treated as
     * a player account (parent can train themselves)", so the public
     * registration form must NOT inherit the 1-18 range — an adult
     * registering themselves is the ordinary case, not an error.
     */
    public function testPublicRegistrationStillAcceptsAnAdultPlayer(): void
    {
        $crawler = $this->client->request('GET', '/join/join-peak-performance');
        $email = sprintf('adult-player-%s@example.test', uniqid());

        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Adult',
            'player_registration[accountLastName]' => 'Player',
            'player_registration[email]' => $email,
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Adult',
            'player_registration[playerDateOfBirth]' => (new \DateTimeImmutable('-40 years'))->format('Y-m-d'),
            'player_registration[playerGender]' => 'unspecified',
        ]);
        $this->client->submit($form);

        self::assertResponseRedirects('/dashboard');
    }

    /**
     * A date of birth in the future is nobody's age, whatever the rest of the
     * form allows.
     */
    public function testPublicRegistrationRejectsAFutureDateOfBirth(): void
    {
        $crawler = $this->client->request('GET', '/join/join-peak-performance');

        $form = $crawler->selectButton('Register')->form([
            'player_registration[accountFirstName]' => 'Future',
            'player_registration[accountLastName]' => 'Person',
            'player_registration[email]' => sprintf('future-person-%s@example.test', uniqid()),
            'player_registration[plainPassword]' => 'correct-horse-battery',
            'player_registration[playerFirstName]' => 'Future',
            'player_registration[playerDateOfBirth]' => (new \DateTimeImmutable('+2 years'))->format('Y-m-d'),
            'player_registration[playerGender]' => 'unspecified',
        ]);
        $this->client->submit($form);

        self::assertResponseStatusCodeSame(422);
        self::assertSelectorExists('#player_registration_playerDateOfBirth[aria-invalid="true"]');
    }
}
