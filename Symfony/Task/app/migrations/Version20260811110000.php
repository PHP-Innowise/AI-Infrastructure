<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Epic-05: adds the two pricing columns AC-05-18 needs to `playlist`
 * ("see a locked playlist with its price ('$50 one-time or 5 tokens')") —
 * columns specs/database-designer-schema.md's own `playlist` table entry
 * (Epic-04) does not list at all.
 *
 * This is a genuine gap, not a judgment call, recorded here and in the
 * coder's final report per the same "widen an existing epic's table"
 * precedent Version20260810181500/Version20260810183000 already established
 * for `share_link`. Owner decision A8 (`requirements-analyst-open-questions.md`)
 * settles that content is sold as a per-playlist one-time purchase, and
 * `payment_record.related_playlist_id` (this epic's own migration,
 * Version20260811100000) already assumes a playlist has a price to charge —
 * but BR-04-8 left "the pricing structure... to be finalized with client",
 * and the schema doc was written before this epic settled the mechanics
 * (Q-05.03, token-package pricing, is the closest analogue and remains
 * genuinely open per that spec's own Open questions — this migration adds
 * only the two amount columns AC-05-18 requires, not a resolution of
 * Q-05.03's broader pricing-strategy question).
 *
 * Modeled on `event`'s own dual-pricing columns (Version20260810120000)
 * for consistency, but deliberately WITHOUT `event`'s separate
 * `*_pricing_enabled` toggles: no AC or business rule anywhere in Epic-04
 * or Epic-05 describes a trainer independently switching either payment
 * method off for a playlist the way AC-02-58/AC-05-30 do for an event —
 * AC-05-18's own wording always shows both options together ("$50
 * one-time or 5 tokens"). A price of 0 in a given unit is this table's own
 * "not offered in that unit" — matching `event.usd_price_minor_units`'s
 * existing `>= 0` (not `> 0`) shape for the same reason. Both columns at 0
 * means free content (BR-04-9's "free content can be suggested without a
 * purchase requirement"), which the paywall check reads directly rather
 * than needing a third boolean.
 *
 * Runs as the owner role (see the `migrate` make target).
 *
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md AC-05-18, A8
 * @see specs/requirements-analyst-epic-04-lp-content-spec.md BR-04-8
 */
final class Version20260811110000 extends AbstractMigration
{
    public function getDescription(): string
    {
        return 'Epic-05: add price_usd_minor_units/price_tokens to playlist (AC-05-18 gap in the Epic-04 schema).';
    }

    public function up(Schema $schema): void
    {
        $this->addSql(<<<'SQL'
            ALTER TABLE playlist
                ADD COLUMN price_usd_minor_units INTEGER DEFAULT 0 NOT NULL,
                ADD COLUMN price_tokens INTEGER DEFAULT 0 NOT NULL
            SQL);
        $this->addSql(<<<'SQL'
            ALTER TABLE playlist
                ADD CONSTRAINT chk_playlist_price_usd_nonnegative CHECK (price_usd_minor_units >= 0),
                ADD CONSTRAINT chk_playlist_price_tokens_nonnegative CHECK (price_tokens >= 0)
            SQL);
    }

    public function down(Schema $schema): void
    {
        $this->addSql('ALTER TABLE playlist DROP CONSTRAINT IF EXISTS chk_playlist_price_tokens_nonnegative');
        $this->addSql('ALTER TABLE playlist DROP CONSTRAINT IF EXISTS chk_playlist_price_usd_nonnegative');
        $this->addSql('ALTER TABLE playlist DROP COLUMN IF EXISTS price_tokens');
        $this->addSql('ALTER TABLE playlist DROP COLUMN IF EXISTS price_usd_minor_units');
    }
}
