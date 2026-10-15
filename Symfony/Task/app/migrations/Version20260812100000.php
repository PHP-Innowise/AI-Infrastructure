<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Canonicalises the skill levels already stored, so the four columns that
 * carry one all spell it the way `App\Identity\Entity\SkillLevel` does.
 *
 * `player_trainer_membership.skill_level` was a free-text field while the
 * segmentation filter beside it, event eligibility and content assignment
 * all compared it exactly. A trainer who typed "intermediate" produced a
 * player no filter could find and, worse, one silently missing from every
 * event restricted to "Intermediate". The forms are closed choice lists now;
 * this fixes the rows written before they were.
 *
 * Deliberately narrow: only case and surrounding whitespace are corrected,
 * and only for values that already ARE one of the four. Anything else a
 * trainer once typed is left exactly as it is — inventing a level for it
 * would be guessing at data this migration cannot interpret, and the reading
 * paths now match case-insensitively anyway, so nothing depends on this
 * having run.
 *
 * No schema change: the columns stay VARCHAR/TEXT[]. Q-01.01 (whether the
 * four are the final vocabulary or trainers may define their own) is still
 * open with the client, and a CHECK constraint would answer it for them.
 */
final class Version20260812100000 extends AbstractMigration
{
    private const array LEVELS = ['Beginner', 'Intermediate', 'Advanced', 'Elite'];

    public function getDescription(): string
    {
        return 'Canonicalise stored skill levels to their documented spelling.';
    }

    public function up(Schema $schema): void
    {
        foreach (self::LEVELS as $level) {
            // Scalar columns: one UPDATE per level, matched case- and
            // whitespace-insensitively.
            $this->addSql(
                'UPDATE player_trainer_membership SET skill_level = :level
                  WHERE skill_level IS NOT NULL
                    AND skill_level <> :level
                    AND lower(btrim(skill_level)) = lower(:level)',
                ['level' => $level],
            );

            $this->addSql(
                'UPDATE playlist_assignment SET target_skill_level = :level
                  WHERE target_skill_level IS NOT NULL
                    AND target_skill_level <> :level
                    AND lower(btrim(target_skill_level)) = lower(:level)',
                ['level' => $level],
            );

            // Array columns: rebuilt element by element, so an entry that is
            // not one of the four survives untouched alongside one that is.
            foreach (['event' => 'skill_levels', 'playlist' => 'filter_skill_levels'] as $table => $column) {
                $this->addSql(
                    sprintf(
                        'UPDATE %1$s SET %2$s = (
                             SELECT array_agg(CASE WHEN lower(btrim(element)) = lower(:level) THEN :level ELSE element END ORDER BY ordinality)
                               FROM unnest(%2$s) WITH ORDINALITY AS t(element, ordinality)
                         )
                         WHERE %2$s IS NOT NULL
                           AND EXISTS (
                             SELECT 1 FROM unnest(%2$s) AS element
                              WHERE lower(btrim(element)) = lower(:level) AND element <> :level
                           )',
                        $table,
                        $column,
                    ),
                    ['level' => $level],
                );
            }
        }
    }

    public function down(Schema $schema): void
    {
        // Irreversible by nature: the capitalisation a trainer originally
        // typed is not recorded anywhere, and restoring a guess would be
        // worse than leaving the corrected value in place. Nothing depends
        // on the correction — every reading path matches case-insensitively.
        $this->throwIrreversibleMigrationException(
            'Skill-level canonicalisation cannot be undone: the original capitalisation is not retained.',
        );
    }
}
