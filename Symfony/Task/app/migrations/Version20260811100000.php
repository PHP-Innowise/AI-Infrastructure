<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * M4 — Billing (Epic-05): the token ledger, payment records, token
 * packages, subscription entitlements, the trainer's own Stripe Connect
 * settings, and the three Stripe-adjacent global tables.
 *
 * Runs as the owner role (see the `migrate` make target). The application
 * role holds no DDL, which is what keeps Row-Level Security meaningful.
 *
 * **Three deviations from specs/database-designer-schema.md's literal text**,
 * all forced rather than chosen, recorded here and in the coder's final
 * report — matching the precedent Version20260810190000 and
 * Version20260810181500 already established for this kind of conflict:
 *
 * 1. **`trainer_billing_settings` is created here, not in an Epic-01
 *    migration.** The schema doc files this table's own column table under
 *    a heading literally named "### Identity module" (line 624), which
 *    would suggest Epic-01 territory. But that same document's own
 *    "Migration ordering" section places its creation in "M4 — Billing"
 *    explicitly, by name, in the same sentence as `payment_record` and
 *    `token_entry` — and architecture's module map states outright that
 *    Billing "Owns: ... Connect settings" (the Stripe Connect account id,
 *    onboarding status, fee rate and prices this table holds). The
 *    migration-ordering section and the module map agree with each other
 *    and disagree with the column table's heading; this migration follows
 *    the two that agree. The table's own trainer-scoped-tables-manifest
 *    grouping (`config/tenancy/trainer_scoped_tables.txt`, "Identity /
 *    Platform (Epic-01)") is left untouched — it groups by WHEN a table was
 *    pre-declared safe to exist, not by which module owns it, and moving it
 *    would not change any behavior the startup gate checks.
 * 2. **`stripe_event_receipt` gains a `raw_payload TEXT NOT NULL` column
 *    beyond the schema doc's own six-column listing for this table** (id,
 *    stripe_event_id, event_type, received_at, processed_at,
 *    processing_error — no payload column named anywhere). This is not
 *    optional: specs/api-designer-spec.md's own "Stripe webhook contract"
 *    (steps 3, 5, 6) is explicit that the receipt row is inserted BEFORE the
 *    event's business meaning is parsed, that the dispatched message carries
 *    only the receipt's id, and that "the handler re-reads rawPayload from
 *    the persisted row rather than trusting a payload that traveled through
 *    the queue." Without a persisted payload column there is nothing for
 *    that re-read to find and the entire async webhook design in that same
 *    document is unbuildable. No `status` column is added alongside it,
 *    unlike that same prose's "status: 'pending'" phrasing — the schema
 *    doc's existing `processed_at IS NULL` / `processing_error IS NOT NULL`
 *    pair already represents all three states (pending / processed-ok /
 *    processed-failed) without a redundant third column.
 * 3. **`fk_payment_record_rsvp` and `fk_rsvp_payment_record` are both
 *    `ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED`**, which the
 *    schema doc's own per-table listings do not state (it does not use
 *    `NO ACTION` anywhere, only `RESTRICT`/`CASCADE`/`SET NULL`). `rsvp`
 *    and `payment_record` reference each other (`rsvp.payment_record_id`
 *    and `payment_record.related_rsvp_id`), and two tables whose foreign
 *    keys point at each other have no valid linear deletion order at all
 *    under Postgres's default (immediate, per-statement) constraint
 *    checking — confirmed directly, not theoretically: Doctrine's fixture
 *    `ORMPurger` (`doctrine:fixtures:load`, which `make test`/`make seed`
 *    both depend on) failed with `SQLSTATE[23503]` in BOTH directions in
 *    turn once `Rsvp::$paymentRecord` became a real ORM relation (see that
 *    entity's own docblock for why it had to).
 *
 *    Deferring both constraints to transaction-commit time — the same
 *    technique `playlist_item`'s own DEFERRABLE unique constraint already
 *    uses in this codebase (Version20260810190000), there for an unrelated
 *    same-transaction-reordering reason — is necessary but, on its own,
 *    NOT sufficient: `doctrine/data-fixtures`' `ORMExecutor` wraps the
 *    whole purge in one transaction (`EntityManager::wrapInTransaction()`),
 *    so `DEFERRABLE INITIALLY DEFERRED` alone should defer the check to
 *    that transaction's COMMIT, by which point the purge has emptied both
 *    tables. It did not: with `ON DELETE RESTRICT`, Postgres raised
 *    `SQLSTATE[23503]` synchronously on the `DELETE FROM rsvp` statement
 *    anyway, even with `condeferred = t` confirmed via `pg_constraint`.
 *    Per PostgreSQL's own documentation of `ON DELETE` actions: "the
 *    essential difference between [`NO ACTION`] and [`RESTRICT`] is that
 *    `NO ACTION` allows the check to be deferred until later in the
 *    transaction, whereas `RESTRICT` does not" — `RESTRICT` forces an
 *    immediate check on the delete-triggered direction regardless of the
 *    constraint's own `DEFERRABLE INITIALLY DEFERRED` clause. Reproduced
 *    directly with a hand-run `BEGIN; DELETE FROM rsvp; DELETE FROM
 *    payment_record;` against this constraint before and after switching
 *    it to `NO ACTION`, isolating the fix from Doctrine/ORMPurger
 *    entirely. Every other FK in this schema keeps `RESTRICT`, its
 *    ordinary meaning ("block the delete, always") being exactly what is
 *    wanted everywhere there is no cycle to break.
 *
 *    A structurally identical cycle was found between `payment_record`
 *    and `subscription_entitlement` while diagnosing this
 *    (`subscription_entitlement.payment_record_id` NOT NULL/UNIQUE, plus a
 *    `payment_record.related_subscription_entitlement_id` pointing back)
 *    and was not given the same DEFERRABLE/`NO ACTION` treatment — it was
 *    removed instead. `related_subscription_entitlement_id` was never
 *    read or written outside `PaymentRecord` itself, was not even wired
 *    into `chk_payment_record_related_shape`'s per-type shape check (a
 *    latent completeness gap, moot now that the column is gone), and
 *    added no information `subscription_entitlement.payment_record_id`
 *    (the required, unique direction, already necessary since an
 *    entitlement cannot exist before the payment that creates it) did not
 *    already carry. See `PaymentRecord::TYPE_PLAYER_SUBSCRIPTION`'s own
 *    docblock.
 *
 * RLS session variable: `app.current_trainer`, matching
 * `App\Platform\Tenancy\TenantContext::SESSION_VARIABLE` and every migration
 * since Epic-01 (NOT `app.current_trainer_id}`, the naming mismatch every
 * prior migration's own docblock already notes).
 *
 * **I7 (append-only)**: `REVOKE UPDATE, DELETE ON token_entry FROM pp_app`
 * is applied at the end of this migration, same pattern as
 * `audit_log_entry` (Version20260810090000). `docker/postgres/grant-roles.sh`
 * is updated in the same commit to re-narrow `token_entry` on every
 * `make test` run too — that script's own comment already named
 * `token_entry` as the next table needing this, written ahead of this epic.
 *
 * Attaches every FK deferred from M1–M3, now that both sides exist:
 * `child_approval_request.requested_token_package_id` -> `token_package(id)`;
 * `rsvp.payment_record_id` -> `payment_record(id)`;
 * `playlist_access_grant.payment_record_id` -> `payment_record(id)` NOT NULL
 * (tightened from nullable-during-creation, per that entity's own docblock
 * and specs/database-designer-schema.md "Migration ordering").
 * `payment_record.related_rsvp_id` and `related_playlist_id` are attached
 * directly in this table's own CREATE TABLE below (both target tables
 * already exist by M4); `related_form_submission_id` stays a deferred,
 * unconstrained column (Forms/M7 does not exist yet). `payment_record` has
 * no `related_subscription_entitlement_id` column — see point 3 above and
 * `PaymentRecord::TYPE_PLAYER_SUBSCRIPTION`'s own docblock for why a
 * `player_subscription` row is linked to its entitlement only through
 * `subscription_entitlement.payment_record_id`, never the reverse.
 * `token_entry.referral_id` is created as a deferred, unconstrained column
 * (Growth/M5 does not exist yet).
 *
 * @see specs/database-designer-schema.md "Billing module (Epic-05)", "The token and payment ledger", "Money and the platform fee"
 * @see specs/architect-architecture.md "The token and payment ledger", "Payment records"
 * @see specs/requirements-analyst-epic-05-payments-tokens-spec.md
 */
final class Version20260811100000 extends AbstractMigration
{
    /**
     * Trainer-scoped tables created here, all getting the standard single
     * USING+WITH CHECK policy — no widened-read table like `playlist` in
     * this epic.
     */
    private const STANDARD_TRAINER_SCOPED_TABLES = [
        'token_package',
        'payment_record',
        'token_entry',
        'token_balance',
        'subscription_entitlement',
        'entitlement_coverage',
        'trainer_billing_settings',
    ];

    public function getDescription(): string
    {
        return 'Epic-05: token ledger, payment records, token packages, subscription entitlements, Stripe Connect settings';
    }

    public function up(Schema $schema): void
    {
        $this->abortIf(
            !$this->connection->getDatabasePlatform() instanceof \Doctrine\DBAL\Platforms\PostgreSQLPlatform,
            'PracticePerfect targets PostgreSQL only: Row-Level Security is load-bearing.',
        );

        // subscription_entitlement's EXCLUDE USING gist needs an equality
        // operator class for bigint, which GiST has no native support for.
        $this->addSql('CREATE EXTENSION IF NOT EXISTS btree_gist');

        // --- trainer_billing_settings ----------------------------------------
        // PK doubles as the FK/tenant key (1-to-1-with-trainer, see this
        // migration's own docblock and the schema doc's own note on this
        // table: "this table's PK *is* its own tenant key").
        $this->addSql(<<<'SQL'
            CREATE TABLE trainer_billing_settings (
                trainer_id BIGINT NOT NULL,
                stripe_connect_account_id VARCHAR(255) DEFAULT NULL,
                stripe_connect_onboarding_status VARCHAR(16) DEFAULT 'pending' NOT NULL,
                platform_fee_basis_points SMALLINT DEFAULT 500 NOT NULL,
                monthly_subscription_price_minor_units INTEGER DEFAULT 1500 NOT NULL,
                token_price_minor_units INTEGER DEFAULT 1000 NOT NULL,
                player_subscription_price_minor_units INTEGER DEFAULT NULL,
                payout_schedule VARCHAR(16) DEFAULT 'monthly' NOT NULL,
                updated_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(trainer_id),
                CONSTRAINT fk_tbs_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT uniq_tbs_stripe_connect_account UNIQUE (stripe_connect_account_id),
                CONSTRAINT chk_tbs_onboarding_status CHECK (stripe_connect_onboarding_status IN ('pending', 'complete', 'incomplete')),
                CONSTRAINT chk_tbs_fee_bp_range CHECK (platform_fee_basis_points BETWEEN 0 AND 10000),
                CONSTRAINT chk_tbs_payout_schedule CHECK (payout_schedule IN ('monthly', 'weekly'))
            )
            SQL);

        // --- token_package ----------------------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE token_package (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                label VARCHAR(100) NOT NULL,
                token_count INTEGER NOT NULL,
                price_minor_units INTEGER NOT NULL,
                is_active BOOLEAN DEFAULT true NOT NULL,
                created_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_token_package_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT chk_token_package_count CHECK (token_count > 0),
                CONSTRAINT chk_token_package_price CHECK (price_minor_units > 0)
            )
            SQL);
        $this->addSql('CREATE INDEX idx_token_package_trainer_active ON token_package (trainer_id, is_active)');

        // --- payment_record -----------------------------------------------------
        // related_rsvp_id / related_playlist_id are attached directly:
        // both target tables already exist by M4 (Scheduling/Content ran
        // first). related_form_submission_id stays deferred, unconstrained
        // (Forms/M7 does not exist yet) — see this migration's own
        // docblock. No related_subscription_entitlement_id column exists;
        // see the docblock's point 3.
        $this->addSql(<<<'SQL'
            CREATE TABLE payment_record (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                type VARCHAR(20) NOT NULL,
                payer_account_id BIGINT DEFAULT NULL,
                contact_name VARCHAR(255) NOT NULL,
                contact_email CITEXT NOT NULL,
                contact_phone VARCHAR(32) DEFAULT NULL,
                payment_method VARCHAR(8) NOT NULL,
                status VARCHAR(16) DEFAULT 'pending' NOT NULL,
                amount_minor_units INTEGER NOT NULL,
                fee_rate_basis_points SMALLINT DEFAULT NULL,
                platform_fee_minor_units INTEGER DEFAULT 0 NOT NULL,
                stripe_payment_intent_id VARCHAR(255) DEFAULT NULL,
                stripe_charge_id VARCHAR(255) DEFAULT NULL,
                stripe_refund_id VARCHAR(255) DEFAULT NULL,
                refunds_payment_record_id BIGINT DEFAULT NULL,
                related_token_package_id BIGINT DEFAULT NULL,
                related_rsvp_id BIGINT DEFAULT NULL,
                related_playlist_id BIGINT DEFAULT NULL,
                related_form_submission_id BIGINT DEFAULT NULL,
                created_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                updated_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_payment_record_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT fk_payment_record_payer FOREIGN KEY (payer_account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT fk_payment_record_refunds FOREIGN KEY (refunds_payment_record_id)
                    REFERENCES payment_record (id) ON DELETE RESTRICT,
                CONSTRAINT fk_payment_record_token_package FOREIGN KEY (related_token_package_id)
                    REFERENCES token_package (id) ON DELETE RESTRICT,
                CONSTRAINT fk_payment_record_rsvp FOREIGN KEY (related_rsvp_id)
                    REFERENCES rsvp (id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED,
                CONSTRAINT fk_payment_record_playlist FOREIGN KEY (related_playlist_id)
                    REFERENCES playlist (id) ON DELETE RESTRICT,
                CONSTRAINT chk_payment_record_type CHECK (type IN ('token_purchase', 'event_rsvp', 'content_purchase', 'player_subscription', 'camp_registration')),
                CONSTRAINT chk_payment_record_method CHECK (payment_method IN ('token', 'card')),
                CONSTRAINT chk_payment_record_status CHECK (status IN ('pending', 'completed', 'failed', 'refunded')),
                CONSTRAINT chk_payment_record_amount CHECK (amount_minor_units > 0),
                CONSTRAINT chk_payment_record_fee_shape CHECK ((refunds_payment_record_id IS NULL) = (fee_rate_basis_points IS NOT NULL)),
                CONSTRAINT chk_payment_record_contact_name CHECK (char_length(trim(contact_name)) > 0),
                CONSTRAINT chk_payment_record_related_shape CHECK (
                    (type = 'token_purchase' AND related_token_package_id IS NOT NULL AND related_rsvp_id IS NULL AND related_playlist_id IS NULL AND related_form_submission_id IS NULL)
                    OR (type = 'event_rsvp' AND related_rsvp_id IS NOT NULL AND related_token_package_id IS NULL AND related_playlist_id IS NULL AND related_form_submission_id IS NULL)
                    OR (type = 'content_purchase' AND related_playlist_id IS NOT NULL AND related_token_package_id IS NULL AND related_rsvp_id IS NULL AND related_form_submission_id IS NULL)
                    OR (type = 'player_subscription' AND related_token_package_id IS NULL AND related_rsvp_id IS NULL AND related_playlist_id IS NULL AND related_form_submission_id IS NULL)
                    OR (type = 'camp_registration' AND related_form_submission_id IS NOT NULL AND related_token_package_id IS NULL AND related_rsvp_id IS NULL AND related_playlist_id IS NULL)
                )
            )
            SQL);
        $this->addSql('CREATE UNIQUE INDEX uniq_payment_record_stripe_pi ON payment_record (stripe_payment_intent_id) WHERE stripe_payment_intent_id IS NOT NULL');
        $this->addSql('CREATE UNIQUE INDEX uniq_payment_record_stripe_charge ON payment_record (stripe_charge_id) WHERE stripe_charge_id IS NOT NULL');
        $this->addSql('CREATE UNIQUE INDEX uniq_payment_record_stripe_refund ON payment_record (stripe_refund_id) WHERE stripe_refund_id IS NOT NULL');
        $this->addSql('CREATE INDEX idx_payment_record_trainer_payer_created ON payment_record (trainer_id, payer_account_id, created_at DESC)');
        $this->addSql("CREATE INDEX idx_payment_record_pending ON payment_record (status) WHERE status = 'pending'");
        $this->addSql('CREATE INDEX idx_payment_record_refunds ON payment_record (refunds_payment_record_id) WHERE refunds_payment_record_id IS NOT NULL');
        $this->addSql('CREATE INDEX idx_payment_record_related_rsvp ON payment_record (related_rsvp_id) WHERE related_rsvp_id IS NOT NULL');
        $this->addSql('CREATE INDEX idx_payment_record_related_playlist ON payment_record (related_playlist_id) WHERE related_playlist_id IS NOT NULL');

        // --- token_entry --------------------------------------------------------
        // Append-only (I7) — REVOKE applied at the end of this migration.
        // referral_id stays a deferred, unconstrained column (Growth/M5 does
        // not exist yet).
        $this->addSql(<<<'SQL'
            CREATE TABLE token_entry (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                parent_account_id BIGINT NOT NULL,
                kind VARCHAR(20) NOT NULL,
                amount INTEGER NOT NULL,
                beneficiary_player_id BIGINT DEFAULT NULL,
                refunds_entry_id BIGINT DEFAULT NULL,
                payment_record_id BIGINT DEFAULT NULL,
                related_event_id BIGINT DEFAULT NULL,
                related_content_item_id BIGINT DEFAULT NULL,
                referral_id BIGINT DEFAULT NULL,
                performed_by_account_id BIGINT DEFAULT NULL,
                description TEXT NOT NULL,
                created_at TIMESTAMP(0) WITH TIME ZONE DEFAULT now() NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_token_entry_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_parent_account FOREIGN KEY (parent_account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_beneficiary FOREIGN KEY (beneficiary_player_id)
                    REFERENCES player_profile (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_refunds FOREIGN KEY (refunds_entry_id)
                    REFERENCES token_entry (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_payment_record FOREIGN KEY (payment_record_id)
                    REFERENCES payment_record (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_event FOREIGN KEY (related_event_id)
                    REFERENCES event (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_content_item FOREIGN KEY (related_content_item_id)
                    REFERENCES content_item (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_entry_performed_by FOREIGN KEY (performed_by_account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT chk_token_entry_kind CHECK (kind IN ('purchase', 'gift', 'referral_reward', 'refund', 'spend', 'adjustment')),
                CONSTRAINT chk_token_entry_amount_nonzero CHECK (amount <> 0),
                CONSTRAINT chk_token_entry_sign_by_kind CHECK (
                    (kind IN ('purchase', 'gift', 'referral_reward', 'refund') AND amount > 0)
                    OR (kind = 'spend' AND amount < 0)
                    OR (kind = 'adjustment' AND amount <> 0)
                ),
                CONSTRAINT chk_token_entry_beneficiary_required CHECK (kind NOT IN ('spend', 'refund') OR beneficiary_player_id IS NOT NULL),
                CONSTRAINT chk_token_entry_refund_reference CHECK ((kind = 'refund') = (refunds_entry_id IS NOT NULL)),
                CONSTRAINT chk_token_entry_description CHECK (char_length(trim(description)) > 0)
            )
            SQL);
        $this->addSql('CREATE UNIQUE INDEX uniq_token_entry_payment_record_kind ON token_entry (payment_record_id, kind) WHERE payment_record_id IS NOT NULL');
        $this->addSql('CREATE INDEX idx_token_entry_trainer_parent_created ON token_entry (trainer_id, parent_account_id, created_at)');
        $this->addSql('CREATE INDEX idx_token_entry_beneficiary ON token_entry (beneficiary_player_id)');
        $this->addSql('CREATE INDEX idx_token_entry_refunds ON token_entry (refunds_entry_id) WHERE refunds_entry_id IS NOT NULL');

        // --- token_balance --------------------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE token_balance (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                parent_account_id BIGINT NOT NULL,
                balance INTEGER DEFAULT 0 NOT NULL,
                updated_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_token_balance_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT fk_token_balance_parent_account FOREIGN KEY (parent_account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT chk_token_balance_nonnegative CHECK (balance >= 0)
            )
            SQL);
        $this->addSql('CREATE UNIQUE INDEX uniq_token_balance_trainer_parent ON token_balance (trainer_id, parent_account_id)');

        // --- subscription_entitlement -----------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE subscription_entitlement (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                parent_account_id BIGINT NOT NULL,
                activation_date DATE NOT NULL,
                window_ends_on DATE GENERATED ALWAYS AS (activation_date + 30) STORED NOT NULL,
                payment_record_id BIGINT NOT NULL,
                created_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_subscription_entitlement_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT fk_subscription_entitlement_parent_account FOREIGN KEY (parent_account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT fk_subscription_entitlement_payment_record FOREIGN KEY (payment_record_id)
                    REFERENCES payment_record (id) ON DELETE RESTRICT,
                CONSTRAINT uniq_subscription_entitlement_payment_record UNIQUE (payment_record_id)
            )
            SQL);
        // No two entitlement windows for the same (trainer, parent) pair may
        // overlap — the GiST equality terms need btree_gist (see the
        // CREATE EXTENSION above) since GiST has no native bigint equality
        // operator class.
        $this->addSql(<<<'SQL'
            ALTER TABLE subscription_entitlement
                ADD CONSTRAINT excl_subscription_entitlement_no_overlap
                EXCLUDE USING gist (
                    trainer_id WITH =,
                    parent_account_id WITH =,
                    daterange(activation_date, window_ends_on, '[]') WITH &&
                )
            SQL);
        $this->addSql('CREATE INDEX idx_subscription_entitlement_lookup ON subscription_entitlement (trainer_id, parent_account_id, activation_date, window_ends_on)');

        // --- entitlement_coverage -----------------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE entitlement_coverage (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                subscription_entitlement_id BIGINT NOT NULL,
                rsvp_id BIGINT NOT NULL,
                covered_at TIMESTAMP(0) WITH TIME ZONE DEFAULT now() NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_entitlement_coverage_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT fk_entitlement_coverage_entitlement FOREIGN KEY (subscription_entitlement_id)
                    REFERENCES subscription_entitlement (id) ON DELETE RESTRICT,
                CONSTRAINT fk_entitlement_coverage_rsvp FOREIGN KEY (rsvp_id)
                    REFERENCES rsvp (id) ON DELETE RESTRICT,
                CONSTRAINT uniq_entitlement_coverage_rsvp UNIQUE (rsvp_id)
            )
            SQL);
        $this->addSql('CREATE INDEX idx_entitlement_coverage_entitlement ON entitlement_coverage (subscription_entitlement_id)');

        // --- platform_subscription (global) -------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE platform_subscription (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                trainer_id BIGINT NOT NULL,
                stripe_subscription_id VARCHAR(255) DEFAULT NULL,
                status VARCHAR(16) DEFAULT 'pending' NOT NULL,
                updated_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_platform_subscription_trainer FOREIGN KEY (trainer_id)
                    REFERENCES trainer (id) ON DELETE RESTRICT,
                CONSTRAINT uniq_platform_subscription_trainer UNIQUE (trainer_id),
                CONSTRAINT uniq_platform_subscription_stripe_id UNIQUE (stripe_subscription_id),
                CONSTRAINT chk_platform_subscription_status CHECK (status IN ('pending', 'active', 'past_due', 'suspended', 'canceled'))
            )
            SQL);
        $this->addSql('CREATE INDEX idx_platform_subscription_status ON platform_subscription (status)');

        // --- stripe_customer_link (global) --------------------------------------
        $this->addSql(<<<'SQL'
            CREATE TABLE stripe_customer_link (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                account_id BIGINT NOT NULL,
                stripe_customer_id VARCHAR(255) NOT NULL,
                default_payment_method_ref VARCHAR(255) DEFAULT NULL,
                created_at TIMESTAMP(0) WITH TIME ZONE NOT NULL,
                PRIMARY KEY(id),
                CONSTRAINT fk_stripe_customer_link_account FOREIGN KEY (account_id)
                    REFERENCES account (id) ON DELETE RESTRICT,
                CONSTRAINT uniq_stripe_customer_link_account UNIQUE (account_id),
                CONSTRAINT uniq_stripe_customer_link_stripe_id UNIQUE (stripe_customer_id)
            )
            SQL);

        // --- stripe_event_receipt (global) --------------------------------------
        // raw_payload is a deliberate addition beyond the schema doc's own
        // column list — see this migration's own docblock, point 2.
        $this->addSql(<<<'SQL'
            CREATE TABLE stripe_event_receipt (
                id BIGINT GENERATED BY DEFAULT AS IDENTITY NOT NULL,
                stripe_event_id VARCHAR(255) NOT NULL,
                event_type VARCHAR(100) NOT NULL,
                raw_payload TEXT NOT NULL,
                received_at TIMESTAMP(0) WITH TIME ZONE DEFAULT now() NOT NULL,
                processed_at TIMESTAMP(0) WITH TIME ZONE DEFAULT NULL,
                processing_error TEXT DEFAULT NULL,
                PRIMARY KEY(id),
                CONSTRAINT uniq_stripe_event_receipt_event_id UNIQUE (stripe_event_id)
            )
            SQL);
        $this->addSql('CREATE INDEX idx_stripe_event_receipt_unprocessed ON stripe_event_receipt (processed_at) WHERE processed_at IS NULL');

        // --- Deferred FKs from M1-M3, now that both sides exist -----------------
        $this->addSql(<<<'SQL'
            ALTER TABLE child_approval_request
                ADD CONSTRAINT fk_car_token_package FOREIGN KEY (requested_token_package_id)
                    REFERENCES token_package (id) ON DELETE RESTRICT
            SQL);
        // DEFERRABLE INITIALLY DEFERRED, matching fk_payment_record_rsvp
        // above — see this migration's own docblock, point 3. rsvp and
        // payment_record reference each other (rsvp.payment_record_id and
        // payment_record.related_rsvp_id), which has no valid linear
        // deletion order at all; Doctrine's fixture ORMPurger
        // (`doctrine:fixtures:load`, which `make test`/`make seed` both
        // depend on) discovered this directly, not theoretically. Deferring
        // both constraints to transaction-commit time (rather than
        // per-statement) is what makes the cycle deletable in one
        // transaction, in any statement order, matching the same technique
        // `playlist_item`'s own DEFERRABLE unique constraint already uses
        // in this codebase (Version20260810190000) for an unrelated
        // same-transaction-reordering reason. `NO ACTION`, not `RESTRICT` —
        // see this migration's own docblock, point 3, for why the two are
        // not interchangeable here.
        $this->addSql(<<<'SQL'
            ALTER TABLE rsvp
                ADD CONSTRAINT fk_rsvp_payment_record FOREIGN KEY (payment_record_id)
                    REFERENCES payment_record (id) ON DELETE NO ACTION DEFERRABLE INITIALLY DEFERRED
            SQL);
        // Tightened NOT NULL first (every existing row, if any, would need a
        // backfill in a populated environment — none exist yet in a fresh
        // one, matching the schema doc's own note on this exact column).
        $this->addSql('ALTER TABLE playlist_access_grant ALTER COLUMN payment_record_id SET NOT NULL');
        $this->addSql(<<<'SQL'
            ALTER TABLE playlist_access_grant
                ADD CONSTRAINT fk_playlist_access_grant_payment_record FOREIGN KEY (payment_record_id)
                    REFERENCES payment_record (id) ON DELETE RESTRICT
            SQL);

        $this->applyRowLevelSecurity();

        // I7 / BR-05-... "Entry immutability": append-only is a database
        // privilege, not a code convention — same pattern as
        // audit_log_entry (Version20260810090000).
        $this->addSql('REVOKE UPDATE, DELETE ON token_entry FROM pp_app');
    }

    private function applyRowLevelSecurity(): void
    {
        foreach (self::STANDARD_TRAINER_SCOPED_TABLES as $table) {
            $this->addSql(sprintf('ALTER TABLE %s ENABLE ROW LEVEL SECURITY', $table));

            $this->addSql(sprintf(
                <<<'SQL'
                    CREATE POLICY %1$s_tenant_isolation ON %1$s
                        USING (trainer_id = NULLIF(current_setting('app.current_trainer', true), '')::bigint)
                        WITH CHECK (trainer_id = NULLIF(current_setting('app.current_trainer', true), '')::bigint)
                    SQL,
                $table,
            ));
        }
    }

    public function down(Schema $schema): void
    {
        $this->addSql('ALTER TABLE playlist_access_grant DROP CONSTRAINT IF EXISTS fk_playlist_access_grant_payment_record');
        $this->addSql('ALTER TABLE playlist_access_grant ALTER COLUMN payment_record_id DROP NOT NULL');
        $this->addSql('ALTER TABLE rsvp DROP CONSTRAINT IF EXISTS fk_rsvp_payment_record');
        $this->addSql('ALTER TABLE child_approval_request DROP CONSTRAINT IF EXISTS fk_car_token_package');

        foreach (self::STANDARD_TRAINER_SCOPED_TABLES as $table) {
            $this->addSql(sprintf('DROP POLICY IF EXISTS %1$s_tenant_isolation ON %1$s', $table));
        }

        $this->addSql('DROP TABLE IF EXISTS stripe_event_receipt');
        $this->addSql('DROP TABLE IF EXISTS stripe_customer_link');
        $this->addSql('DROP TABLE IF EXISTS platform_subscription');
        $this->addSql('DROP TABLE IF EXISTS entitlement_coverage');
        $this->addSql('DROP TABLE IF EXISTS subscription_entitlement');
        $this->addSql('DROP TABLE IF EXISTS token_balance');
        $this->addSql('DROP TABLE IF EXISTS token_entry');
        $this->addSql('DROP TABLE IF EXISTS payment_record');
        $this->addSql('DROP TABLE IF EXISTS token_package');
        $this->addSql('DROP TABLE IF EXISTS trainer_billing_settings');
    }
}
