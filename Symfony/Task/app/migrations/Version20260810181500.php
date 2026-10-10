<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Epic-03: widens `share_link.link_type` to admit a third value,
 * `coach_player_invite` (US-03.11 "Coach Invites Player via ShareLink",
 * AC-03-50..53).
 *
 * specs/api-designer-spec.md "Identity module" Decisions row "`/invite/{code}`
 * reuse" settles the open question `specs/requirements-analyst-epic-03-crm-players-spec.md`
 * itself raises (Open questions, "Cross-epic gap, new ShareLink type"): the
 * coach-issued player invite reuses `/invite/{code}`'s existing URL shape
 * "branching on `ShareLink.type`" — which requires `ShareLink.type` to carry
 * a THIRD, distinct value from Epic-01's two (`static_player`, `unique_coach`),
 * since "only the stored `ShareLink.type` differs" between the two invite
 * flows sharing that URL. `specs/database-designer-schema.md`'s own
 * `share_link` table entry predates this cross-epic reuse decision and only
 * lists the original two values — this migration is the schema-level
 * completion of a decision made in a sibling spec, not a deviation from the
 * database design; recorded in the coder's final report.
 *
 * Unlike `unique_coach` (BR-01-15: mandatory target email, mandatory 7-day
 * expiry), the coach-player invite has no stated email requirement
 * (`InvitePlayerType`'s recipient email is optional per
 * `specs/api-designer-spec.md:421`) and no stated expiry anywhere in the
 * epic — modeled here as single-use (`max_uses = 1`, matching AC-03-52's
 * "status (pending, accepted)" reading as a one-shot invitation) with no
 * forced expiry, since inventing one would not be traceable to any AC or
 * business rule.
 *
 * Runs as the owner role (see the `migrate` make target). Postgres has no
 * `ALTER CONSTRAINT` for a CHECK's own expression — drop and recreate is the
 * only route, matching Version20260810150000's own precedent.
 *
 * @see specs/requirements-analyst-epic-03-crm-players-spec.md AC-03-50..53, Open questions
 * @see specs/api-designer-spec.md "Identity module" Decisions, "`/invite/{code}` reuse"
 */
final class Version20260810181500 extends AbstractMigration
{
    public function getDescription(): string
    {
        return 'Widen share_link.link_type to admit coach_player_invite (AC-03-50..53).';
    }

    public function up(Schema $schema): void
    {
        // 'coach_player_invite' is 20 characters — the original VARCHAR(16)
        // (sized for 'static_player'/'unique_coach' alone) is too narrow.
        $this->addSql('ALTER TABLE share_link ALTER COLUMN link_type TYPE VARCHAR(24)');

        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT chk_share_link_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_type
                CHECK (link_type IN ('static_player','unique_coach','coach_player_invite'))
            SQL);

        // No email, no expiry, but exactly one use — the shape check mirrors
        // chk_share_link_coach_shape/chk_share_link_static_shape's own
        // pattern for the two pre-existing types.
        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_coach_player_invite_shape CHECK (
                    (link_type <> 'coach_player_invite')
                    OR (expires_at IS NULL AND max_uses = 1)
                )
            SQL);
    }

    public function down(Schema $schema): void
    {
        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT IF EXISTS chk_share_link_coach_player_invite_shape');
        $this->addSql('ALTER TABLE share_link DROP CONSTRAINT chk_share_link_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE share_link
                ADD CONSTRAINT chk_share_link_type
                CHECK (link_type IN ('static_player','unique_coach'))
            SQL);
        $this->addSql('ALTER TABLE share_link ALTER COLUMN link_type TYPE VARCHAR(16)');
    }
}
