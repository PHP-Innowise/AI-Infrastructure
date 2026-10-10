<?php

declare(strict_types=1);

namespace App\Identity\Entity;

/**
 * The four skill levels, in one place, because five places used to disagree
 * about them.
 *
 * `player_trainer_membership.skill_level` was a free-text field. The
 * segmentation filter beside it (US-03.06 "Skill Level: Beginner,
 * Intermediate, Advanced, Elite") was a closed list matched with `=`, event
 * eligibility used `in_array(..., strict)`, and content assignment used
 * `===`. So a trainer who typed "intermediate" got a saved profile that no
 * filter could find and, worse, a player silently excluded from every event
 * restricted to "Intermediate" — no error on either side, just a shorter
 * calendar. Manual testing reproduced exactly that: same player, same event,
 * only the capitalisation changed, and the event disappeared.
 *
 * This class is deliberately NOT a backed enum. The column is a nullable
 * VARCHAR(50) that may already hold anything a trainer once typed, and an
 * enum would make reading such a row a hydration failure rather than a
 * display quirk. `canonicalize()` is what every comparison goes through, so
 * legacy spellings keep matching the value they obviously meant while new
 * input is constrained to the list.
 *
 * Q-01.01 ("Beginner, Intermediate, Advanced, Elite, or custom?") is still
 * open with the client, and this does not answer it: the four names below
 * are the ones Epic-03 and Epic-04 already fix in writing. If the answer
 * turns out to be "custom", this list becomes trainer-configurable data and
 * every consumer already reads it from one place.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md US-03.06
 * @see specs/requirements-analyst-epic-04-lppp-content-spec.md AC-04-9
 */
final class SkillLevel
{
    public const string BEGINNER = 'Beginner';
    public const string INTERMEDIATE = 'Intermediate';
    public const string ADVANCED = 'Advanced';
    public const string ELITE = 'Elite';

    /**
     * Ordered as the epics list them — least to most advanced, which is also
     * how every dropdown built from this reads.
     *
     * @var list<string>
     */
    public const array ALL = [
        self::BEGINNER,
        self::INTERMEDIATE,
        self::ADVANCED,
        self::ELITE,
    ];

    /**
     * `['Beginner' => 'Beginner', ...]` for Symfony's ChoiceType, where the
     * label and the stored value are the same word.
     *
     * @return array<string, string>
     */
    public static function choices(): array
    {
        return array_combine(self::ALL, self::ALL);
    }

    /**
     * The canonical spelling of a stored value, or null when it is not one
     * of the four at all.
     *
     * Case- and whitespace-insensitive, because that is the whole class of
     * near-miss a free-text field produced. A value that matches nothing —
     * a trainer's "Int." or "rec league" — returns null rather than being
     * invented into a level it never was; it stays in the column, visible on
     * the profile, and simply matches no filter, which is honest.
     */
    public static function canonicalize(?string $value): ?string
    {
        if (null === $value) {
            return null;
        }

        $normalized = strtolower(trim($value));

        foreach (self::ALL as $level) {
            if (strtolower($level) === $normalized) {
                return $level;
            }
        }

        return null;
    }

    /**
     * Whether two stored skill levels mean the same thing. Falls back to an
     * exact comparison when neither side is one of the four, so two rows
     * carrying the same custom word still match each other.
     */
    public static function matches(?string $one, ?string $other): bool
    {
        if (null === $one || null === $other) {
            return false;
        }

        $canonicalOne = self::canonicalize($one);
        $canonicalOther = self::canonicalize($other);

        if (null !== $canonicalOne || null !== $canonicalOther) {
            return $canonicalOne === $canonicalOther;
        }

        return strtolower(trim($one)) === strtolower(trim($other));
    }
}
