<?php

declare(strict_types=1);

namespace App\Tests\Identity;

use App\Identity\Entity\SkillLevel;
use PHPUnit\Framework\Attributes\DataProvider;
use PHPUnit\Framework\TestCase;

/**
 * The vocabulary Epic-03's segmentation filter (US-03.06) and Epic-04's
 * assignment targeting both name, and the comparison every consumer of it
 * goes through.
 */
final class SkillLevelTest extends TestCase
{
    public function testTheFourLevelsAreTheOnesTheEpicsName(): void
    {
        self::assertSame(['Beginner', 'Intermediate', 'Advanced', 'Elite'], SkillLevel::ALL);
    }

    /**
     * The near-misses a free-text field produced for two epics' worth of
     * screens.
     */
    #[DataProvider('canonicalizationProvider')]
    public function testCanonicalization(?string $stored, ?string $expected): void
    {
        self::assertSame($expected, SkillLevel::canonicalize($stored));
    }

    /**
     * @return iterable<string, array{?string, ?string}>
     */
    public static function canonicalizationProvider(): iterable
    {
        yield 'already canonical' => ['Intermediate', 'Intermediate'];
        yield 'lowercase' => ['intermediate', 'Intermediate'];
        yield 'uppercase' => ['ELITE', 'Elite'];
        yield 'padded' => ['  Beginner ', 'Beginner'];
        yield 'null' => [null, null];
        yield 'empty' => ['', null];
        yield 'not a level at all' => ['rec league', null];
    }

    public function testTwoSpellingsOfTheSameLevelMatch(): void
    {
        self::assertTrue(SkillLevel::matches('intermediate', 'Intermediate'));
        self::assertTrue(SkillLevel::matches('Intermediate', 'INTERMEDIATE'));
        self::assertFalse(SkillLevel::matches('Beginner', 'Advanced'));
    }

    /**
     * A value outside the four is not silently promoted into one, and is not
     * made to match a level it never was — but two rows carrying the same
     * custom word still recognise each other, since the column may hold one
     * from before this list existed.
     */
    public function testACustomValueMatchesOnlyItself(): void
    {
        self::assertFalse(SkillLevel::matches('rec league', 'Beginner'));
        self::assertFalse(SkillLevel::matches('Beginner', 'rec league'));
        self::assertTrue(SkillLevel::matches('rec league', 'Rec League'));
    }

    public function testNullNeverMatches(): void
    {
        self::assertFalse(SkillLevel::matches(null, 'Beginner'), 'A player with no level set matches no restriction.');
        self::assertFalse(SkillLevel::matches('Beginner', null));
        self::assertFalse(SkillLevel::matches(null, null));
    }
}
