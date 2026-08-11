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

export PGPASSWORD="${DB_OWNER_PASSWORD}"

psql -h "${DB_HOST:-db}" -p "${DB_PORT:-5432}" \
     -U "${DB_OWNER_USERNAME}" -d "$DB" -v ON_ERROR_STOP=1 <<-SQL
    GRANT USAGE ON SCHEMA public TO "${DB_USERNAME}", "${DB_CROSSING_USERNAME}";
    REVOKE CREATE ON SCHEMA public FROM "${DB_USERNAME}", "${DB_CROSSING_USERNAME}";

    ALTER DEFAULT PRIVILEGES FOR ROLE "${DB_OWNER_USERNAME}" IN SCHEMA public
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO "${DB_USERNAME}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${DB_OWNER_USERNAME}" IN SCHEMA public
        GRANT USAGE, SELECT ON SEQUENCES TO "${DB_USERNAME}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${DB_OWNER_USERNAME}" IN SCHEMA public
        GRANT SELECT ON TABLES TO "${DB_CROSSING_USERNAME}";
    ALTER DEFAULT PRIVILEGES FOR ROLE "${DB_OWNER_USERNAME}" IN SCHEMA public
        GRANT SELECT ON SEQUENCES TO "${DB_CROSSING_USERNAME}";

    -- Anything created before these defaults were set.
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO "${DB_USERNAME}";
    GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO "${DB_USERNAME}";
    GRANT SELECT ON ALL TABLES IN SCHEMA public TO "${DB_CROSSING_USERNAME}";
SQL

echo "[grants] applied to ${DB} for ${DB_USERNAME} and ${DB_CROSSING_USERNAME}"
