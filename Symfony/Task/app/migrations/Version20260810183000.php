<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Epic-03 (optional MVP): widens `share_link.link_type` to admit a fourth
 * value, `unique_player_invite` — AC-03-61, "Trainer can generate unique,
 * one-time links per player/parent."
 *
 * Same shape as `coach_player_invite` (Version20260810181500): single-use,
 * no expiry, optional target email — kept as its own type rather than a
 * reuse, purely so a link's own `link_type` stays an honest record of who
 * may create it (a trainer here, a coach there) — see `ShareLink`'s own
 * docblock. `VARCHAR(24)` (already widened by Version20260810181500) already
 * fits `unique_player_invite` (21 characters), so no further column
 * widening is needed here.
 *
 * Runs as the owner role (see the `migrate` make target).
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-61, AC-03-63, BR-03-21
 */
final class Version20260810183000 extends AbstractMigration
{
    public function getDescription(): string
    {
        return 'Widen share_link.link_type to admit unique_player_invite (AC-03-61, optional MVP).';
    }

    public function up(Schema $schema): void
    {
        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT chk_share_link_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_type
                CHECK (link_type IN ('static_player','unique_coach','coach_player_invite','unique_player_invite'))
            SQL);

        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_unique_player_invite_shape CHECK (
                    (link_type <> 'unique_player_invite')
                    OR (expires_at IS NULL AND max_uses = 1)
                )
            SQL);
    }

    public function down(Schema $schema): void
    {
        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT IF EXISTS chk_share_link_unique_player_invite_shape');
        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT chk_share_link_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_type
                CHECK (link_type IN ('static_player','unique_coach','coach_player_invite'))
            SQL);
    }
}
