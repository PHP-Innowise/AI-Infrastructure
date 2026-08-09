<?php

declare(strict_types=1);

namespace DoctrineMigrations;

use Doctrine\DBAL\Schema\Schema;
use Doctrine\Migrations\AbstractMigration;

/**
 * Walking skeleton: the Messenger transport table.
 *
 * The Doctrine transport would normally create this itself via auto_setup, but
 * the application role deliberately holds no DDL — CREATE on schema public is
 * revoked from it, because Row-Level Security is only as good as the privilege
 * separation underneath it. So the table is created here instead, by a
 * migration that runs as the owner role.
 *
 * Table privileges are not granted explicitly: the initdb bootstrap sets
 * ALTER DEFAULT PRIVILEGES for the owner, so anything the owner creates in
 * schema public is already reachable by the application role.
 *
 * This table is NOT trainer-scoped and must not appear in
 * config/tenancy/trainer_scoped_tables.txt.
 *
 * @see specs/architect-architecture.md "How it is wired", "Async scope"
 */
final class Version20260809120000 extends AbstractMigration
{
    public function getDescription(): string
    {
        return 'Create the Messenger doctrine transport table (async and failed queues)';
    }

    public function up(Schema $schema): void
    {
        $this->abortIf(
            !$this->connection->getDatabasePlatform() instanceof \Doctrine\DBAL\Platforms\PostgreSQLPlatform,
            'PracticePerfect targets PostgreSQL only: Row-Level Security is load-bearing.',
        );

        $this->addSql(<<<'SQL'
            CREATE TABLE messenger_messages (
                id BIGSERIAL NOT NULL,
                body TEXT NOT NULL,
                headers TEXT NOT NULL,
                queue_name VARCHAR(190) NOT NULL,
                created_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
                available_at TIMESTAMP(0) WITHOUT TIME ZONE NOT NULL,
                delivered_at TIMESTAMP(0) WITHOUT TIME ZONE DEFAULT NULL,
                PRIMARY KEY(id)
            )
            SQL);

        $this->addSql('CREATE INDEX idx_messenger_messages_queue_name ON messenger_messages (queue_name)');
        $this->addSql('CREATE INDEX idx_messenger_messages_available_at ON messenger_messages (available_at)');
        $this->addSql('CREATE INDEX idx_messenger_messages_delivered_at ON messenger_messages (delivered_at)');

        $this->addSql("COMMENT ON COLUMN messenger_messages.created_at IS '(DC2Type:datetime_immutable)'");
        $this->addSql("COMMENT ON COLUMN messenger_messages.available_at IS '(DC2Type:datetime_immutable)'");
        $this->addSql("COMMENT ON COLUMN messenger_messages.delivered_at IS '(DC2Type:datetime_immutable)'");

        // LISTEN/NOTIFY lets the worker react immediately instead of polling.
        $this->addSql(<<<'SQL'
            CREATE OR REPLACE FUNCTION notify_messenger_messages() RETURNS TRIGGER AS $$
                BEGIN
                    PERFORM pg_notify('messenger_messages', NEW.queue_name::text);
                    RETURN NEW;
                END;
            $$ LANGUAGE plpgsql
            SQL);

        $this->addSql(<<<'SQL'
            CREATE TRIGGER notify_trigger
                AFTER INSERT OR UPDATE ON messenger_messages
                FOR EACH ROW EXECUTE PROCEDURE notify_messenger_messages()
            SQL);
    }

    public function down(Schema $schema): void
    {
        $this->addSql('DROP TRIGGER IF EXISTS notify_trigger ON messenger_messages');
        $this->addSql('DROP FUNCTION IF EXISTS notify_messenger_messages()');
        $this->addSql('DROP TABLE messenger_messages');
    }
}
