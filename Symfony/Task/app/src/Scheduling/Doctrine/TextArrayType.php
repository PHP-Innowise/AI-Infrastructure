<?php

declare(strict_types=1);

namespace App\Scheduling\Doctrine;

use Doctrine\DBAL\Platforms\AbstractPlatform;
use Doctrine\DBAL\Types\Type;

/**
 * Maps `event.skill_levels` and `event.genders` — native PostgreSQL `TEXT[]`
 * columns, per specs/database-designer-schema.md — to `list<string>|null`.
 *
 * Doctrine DBAL ships no native Postgres array type. The schema's own
 * Conventions section states the intended shape ("Doctrine maps each [value
 * vocabulary] to a PHP... type via a custom Doctrine type... so the ORM layer
 * still gets static typing"); this is that custom type, narrowly scoped to
 * the one column shape Epic-02 needs (a flat array of short strings, no
 * nested arrays, no embedded commas/braces expected in practice, though the
 * escaping below handles them correctly regardless).
 */
final class TextArrayType extends Type
{
    public const NAME = 'text_array';

    public function getSQLDeclaration(array $column, AbstractPlatform $platform): string
    {
        return 'TEXT[]';
    }

    /**
     * @return list<string>|null
     */
    public function convertToPHPValue(mixed $value, AbstractPlatform $platform): ?array
    {
        if (null === $value) {
            return null;
        }

        if (\is_array($value)) {
            /** @var list<string> $value */
            return $value;
        }

        $trimmed = trim((string) $value, '{}');

        if ('' === $trimmed) {
            return [];
        }

        $items = str_getcsv($trimmed, ',', '"', '\\');

        return array_map(static fn (mixed $item): string => (string) $item, $items);
    }

    /**
     * @param list<string>|null $value
     */
    public function convertToDatabaseValue(mixed $value, AbstractPlatform $platform): ?string
    {
        if (null === $value) {
            return null;
        }

        $escaped = array_map(
            static fn (string $item): string => '"'.str_replace(['\\', '"'], ['\\\\', '\\"'], $item).'"',
            $value,
        );

        return '{'.implode(',', $escaped).'}';
    }

    public function requiresSQLCommentHint(AbstractPlatform $platform): bool
    {
        return true;
    }
}
