<?php

declare(strict_types=1);

namespace App\Identity\Entity;

/**
 * The gender values a player profile can hold, in one place, for the same
 * reason `SkillLevel` exists: five places handled this value and three of
 * them disagreed.
 *
 * Every form that WRITES a gender — registration, add-a-child, the admin
 * account editor, the form-submission conversion — has always offered
 * `female|male|unspecified`. The two places that READ it did not agree with
 * them:
 *
 *   - the CRM segmentation filter offered `other`, a value no form can
 *     produce, so filtering by it returned nobody, ever;
 *   - the Event Builder's eligibility restriction was free text compared
 *     with `in_array(..., strict)`, so an event restricted to "Female"
 *     excluded every player, whose profile says `female`.
 *
 * Neither failed loudly. Both just returned less than they should have.
 *
 * **An open question this does NOT answer.** Epic-03's filter (US-03.06)
 * names the third bucket "Other", while every form in the product calls it
 * "Prefer not to say" and stores `unspecified`. Those are not the same idea —
 * a third gender is not the same as declining to state one — and the epics
 * never distinguish them. This class keeps the stored value and the wording
 * the forms already use, because that is what the people in the database
 * actually chose; if the client meant a distinct third gender, that is a
 * fourth value and a data question, not a relabelling. Raised, not decided.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md US-03.06
 */
final class Gender
{
    public const string FEMALE = 'female';
    public const string MALE = 'male';
    public const string UNSPECIFIED = 'unspecified';

    /**
     * @var list<string>
     */
    public const array ALL = [self::FEMALE, self::MALE, self::UNSPECIFIED];

    /**
     * Stored value => label. In this direction so a Twig template can build a
     * `<select>` from it with `constant()`, which reaches constants but not
     * static methods.
     *
     * @var array<string, string>
     */
    public const array LABELS = [
        self::FEMALE => 'Female',
        self::MALE => 'Male',
        self::UNSPECIFIED => 'Prefer not to say',
    ];

    /**
     * Label => stored value, which is the direction Symfony's ChoiceType
     * wants.
     *
     * @return array<string, string>
     */
    public static function choices(): array
    {
        return array_flip(self::LABELS);
    }

    /**
     * The canonical stored value, or null when it is not one of the three.
     *
     * Case- and whitespace-insensitive, since the event restriction accepted
     * free text for as long as it was a text field.
     */
    public static function canonicalize(?string $value): ?string
    {
        if (null === $value) {
            return null;
        }

        $normalized = strtolower(trim($value));

        foreach (self::ALL as $gender) {
            if ($gender === $normalized) {
                return $gender;
            }
        }

        return null;
    }

    /**
     * Whether two stored genders mean the same thing. Falls back to an exact,
     * case-insensitive comparison when neither side is one of the three, so a
     * value written before this list existed still matches its own twin.
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
