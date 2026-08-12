<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * M2 — Scheduling (Epic-02), continued: widens `child_approval_request`'s
 * `action_type` CHECK to admit `'rsvp_cancellation'`.
 *
 * AC-02-33 ("If a child account initiates the cancellation, parent approval
 * is required, the same as for a purchase") needs its own action type,
 * distinct from `'rsvp'` (AC-02-23..26's creation gate) — approving a
 * pending RSVP *creation* request and approving a pending RSVP
 * *cancellation* request drive opposite outcomes for the same Rsvp row
 * (RsvpService::completeAfterParentApproval() vs
 * ::completeCancellationAfterApproval()), so the two must be
 * distinguishable at read time, not inferred from context.
 *
 * `child_approval_request` itself is Epic-01's table (Version20260810090000);
 * this only widens a CHECK constraint on it, run as the owner role like every
 * other migration (see the `migrate` make target). Postgres has no
 * `ALTER CONSTRAINT` for a CHECK's own expression — drop and recreate is the
 * only route.
 *
 * @see specs/requirements-analyst-epic-02-event-management-spec.md AC-02-33
 */
final class Version20260810150000 extends AbstractMigration
{
    public function getDescription(): string
    {
        return 'Widen child_approval_request.action_type to admit rsvp_cancellation (AC-02-33).';
    }

    public function up(Schema $schema): void
    {
        $this->addSql('ALTER TABLE child_approval_request DROP CONSTRAINT chk_car_action_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE child_approval_request
                ADD CONSTRAINT chk_car_action_type
                CHECK (action_type IN ('rsvp','rsvp_cancellation','token_purchase','content_purchase'))
            SQL);
    }

    public function down(Schema $schema): void
    {
        $this->addSql('ALTER TABLE child_approval_request DROP CONSTRAINT chk_car_action_type');
        $this->addSql(<<<'SQL'
            ALTER TABLE child_approval_request
                ADD CONSTRAINT chk_car_action_type
                CHECK (action_type IN ('rsvp','token_purchase','content_purchase'))
            SQL);
    }
}
