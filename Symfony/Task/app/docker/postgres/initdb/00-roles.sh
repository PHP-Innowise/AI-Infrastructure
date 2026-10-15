#!/bin/sh
# Bootstrap the three database roles the tenancy model requires. Runs once, on
# first initialisation of the db-data volume, as the owner superuser.
#
#   owner     — owns the schema, runs migrations, holds DDL. Never used by the
#               running application.
#   app       — what php-fpm and the worker connect as. Subject to Row-Level
#               Security: no BYPASSRLS, no superuser, no DDL.
#   crossing  — read-only with BYPASSRLS. Used only by the cross-tenant read
#               service, which returns arrays and has no EntityManager, so it is
#               structurally incapable of producing a managed entity.
#
# See specs/architect-architecture.md, "How it is wired".
set -eu

psql -v ON_ERROR_STOP=1 --username "$POSTGRES_USER" --dbname "$POSTGRES_DB" <<-EOSQL
    CREATE ROLE "${APP_DB_USER}" LOGIN PASSWORD '${APP_DB_PASSWORD}'
        NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS NOINHERIT;

    CREATE ROLE "${CROSSING_DB_USER}" LOGIN PASSWORD '${CROSSING_DB_PASSWORD}'
        NOSUPERUSER NOCREATEDB NOCREATEROLE BYPASSRLS NOINHERIT;

    GRANT CONNECT ON DATABASE "${POSTGRES_DB}" TO "${APP_DB_USER}", "${CROSSING_DB_USER}";
    GRANT USAGE ON SCHEMA public TO "${APP_DB_USER}", "${CROSSING_DB_USER}";

    -- The application role must never hold DDL. Migrations run as the owner.
    REVOKE CREATE ON SCHEMA public FROM "${APP_DB_USER}", "${CROSSING_DB_USER}";

    -- Tables created later by the owner grant these automatically. Per-table
    -- narrowing (the append-only ledger and audit log hold INSERT+SELECT only)
    -- is applied by the migration that creates those tables.
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_USER}" IN SCHEMA public
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO "${APP_DB_USER}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_USER}" IN SCHEMA public
        GRANT USAGE, SELECT ON SEQUENCES TO "${APP_DB_USER}";

    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_USER}" IN SCHEMA public
        GRANT SELECT ON TABLES TO "${CROSSING_DB_USER}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_USER}" IN SCHEMA public
        GRANT SELECT ON SEQUENCES TO "${CROSSING_DB_USER}";
EOSQL

echo "[initdb] roles ${APP_DB_USER} (RLS-bound) and ${CROSSING_DB_USER} (BYPASSRLS, read-only) created"
