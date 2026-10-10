<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Canonicalises the gender values stored on event restrictions, the companion
 * to Version20260812100000's skill levels.
 *
 * `event.genders` was free text while eligibility compared it exactly against
 * a player's own gender, which every form in the product writes as
 * `female|male|unspecified`. A trainer who typed "Female" restricted the
 * event to nobody at all. The field is a choice list now; this fixes what was
 * written before it was.
 *
 * As narrow as its predecessor: case and surrounding whitespace only, and
 * only for entries that already ARE one of the three. Anything else a trainer
 * once typed is left exactly as it is, and the reading path matches
 * case-insensitively anyway, so nothing depends on this having run.
 *
 * `player_profile.gender` is untouched — it has always been written from a
 * closed list, so there is nothing there to correct.
 */
final class Version20260812110000 extends AbstractMigration
{
    private const array GENDERS = ['female', 'male', 'unspecified'];

    public function getDescription(): string
    {
        return 'Canonicalise stored event gender restrictions to their documented spelling.';
    }

    public function up(Schema $schema): void
    {
        foreach (self::GENDERS as $gender) {
            // Rebuilt element by element, so an entry that is not one of the
            // three survives untouched beside one that is.
            $this->addSql(
                <<<'SQL'
                    UPDATE event SET genders = (
                        SELECT array_agg(CASE WHEN lower(btrim(element)) = lower(:gender) THEN :gender ELSE element END ORDER BY ordinality)
                          FROM unnest(genders) WITH ORDINALITY AS t(element, ordinality)
                    )
                    WHERE genders IS NOT NULL
                      AND EXISTS (
                        SELECT 1 FROM unnest(genders) AS element
                         WHERE lower(btrim(element)) = lower(:gender) AND element <> :gender
                      )
                    SQL,
                ['gender' => $gender],
            );
        }
    }

    public function down(Schema $schema): void
    {
        // Irreversible for the same reason as the skill-level canonicalisation:
        // the capitalisation a trainer originally typed is recorded nowhere,
        // and nothing depends on the correction having been applied.
        $this->throwIrreversibleMigrationException(
            'Gender canonicalisation cannot be undone: the original capitalisation is not retained.',
        );
    }
}
