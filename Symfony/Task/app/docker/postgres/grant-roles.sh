#!/bin/sh
# Grant the application and crossing roles their privileges on a database.
#
# The initdb bootstrap does this for the main database, but PostgreSQL default
# privileges are per-database, so any database created afterwards — the test
# database, most importantly — needs the same treatment or the application role
# cannot read a single table in it.
#
# Runs from the php container, which carries psql and the owner credentials.
#   docker compose exec php sh docker/postgres/grant-roles.sh practiceperfect_test
set -eu

DB="${1:?usage: grant-roles.sh <database>}"

export PGPASSWORD="${POSTGRES_OWNER_PASSWORD}"

psql -h "${DB_HOST:-db}" -p "${DB_PORT_INTERNAL:-5432}" \
     -U "${POSTGRES_OWNER_USER}" -d "$DB" -v ON_ERROR_STOP=1 <<-SQL
    GRANT USAGE ON SCHEMA public TO "${APP_DB_USER}", "${CROSSING_DB_USER}";
    REVOKE CREATE ON SCHEMA public FROM "${APP_DB_USER}", "${CROSSING_DB_USER}";

    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_OWNER_USER}" IN SCHEMA public
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO "${APP_DB_USER}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_OWNER_USER}" IN SCHEMA public
        GRANT USAGE, SELECT ON SEQUENCES TO "${APP_DB_USER}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_OWNER_USER}" IN SCHEMA public
        GRANT SELECT ON TABLES TO "${CROSSING_DB_USER}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${POSTGRES_OWNER_USER}" IN SCHEMA public
        GRANT SELECT ON SEQUENCES TO "${CROSSING_DB_USER}";

    -- Anything already created before these defaults were set.
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO "${APP_DB_USER}";
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO "${APP_DB_USER}";
    GRANT SELECT ON ALL TABLES IN SCHEMA public TO "${CROSSING_DB_USER}";
SQL

echo "[grants] applied to ${DB} for ${APP_DB_USER} and ${CROSSING_DB_USER}"
