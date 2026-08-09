#!/bin/sh
# PracticePerfect container entrypoint and startup gate.
#
# Row-Level Security is the one tenancy layer that fails silently when it is
# misconfigured: if the application connects as the table owner, or as a role
# holding BYPASSRLS, every policy is skipped and nothing complains. That is the
# single failure RLS cannot self-detect, so this gate refuses to start the
# container rather than serve one trainer's rows to another.
#
# See specs/architect-architecture.md, "The RLS disagreement, resolved" and
# "Risks".
set -eu

APP_UID="${APP_UID:-1000}"
APP_GID="${APP_GID:-1000}"

DB_HOST="${DB_HOST:-db}"
DB_PORT_INTERNAL="${DB_PORT_INTERNAL:-5432}"
DB_NAME="${POSTGRES_DB:-practiceperfect}"
DB_OWNER="${POSTGRES_OWNER_USER:-pp_owner}"
DB_APP_USER="${APP_DB_USER:-pp_app}"
DB_APP_PASSWORD="${APP_DB_PASSWORD:-}"

log()  { printf '[entrypoint] %s\n' "$*" >&2; }
fail() { printf '[entrypoint] FATAL: %s\n' "$*" >&2; exit 1; }

# --- Writable runtime directories -------------------------------------------
# Named volumes mount root-owned; the workers run as the host user.
for dir in /app/var /app/public/uploads; do
    if [ -d "$dir" ]; then
        chown -R "${APP_UID}:${APP_GID}" "$dir" 2>/dev/null || true
    fi
done

# --- Wait for PostgreSQL -----------------------------------------------------
log "waiting for postgres at ${DB_HOST}:${DB_PORT_INTERNAL}"
attempt=0
until pg_isready -h "$DB_HOST" -p "$DB_PORT_INTERNAL" -U "$DB_APP_USER" -d "$DB_NAME" >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    [ "$attempt" -ge 60 ] && fail "postgres did not become ready within 60 attempts"
    sleep 1
done
log "postgres is ready"

# --- Startup gate ------------------------------------------------------------
export PGPASSWORD="$DB_APP_PASSWORD"
psql_app() {
    psql -h "$DB_HOST" -p "$DB_PORT_INTERNAL" -U "$DB_APP_USER" -d "$DB_NAME" \
         -t -A -v ON_ERROR_STOP=1 -c "$1"
}

# Gate 1 — the application role must not be the role that owns the schema.
# An owner is exempt from its own policies unless FORCE ROW LEVEL SECURITY is
# set, so connecting as the owner silently disables tenancy isolation.
if [ "$DB_APP_USER" = "$DB_OWNER" ]; then
    fail "the application is configured to connect as the schema owner '${DB_OWNER}'. Row-Level Security would not apply. Set APP_DB_USER to the unprivileged application role."
fi

# Gate 2 — the application role must not hold BYPASSRLS or superuser.
role_flags="$(psql_app "SELECT rolbypassrls::text || ' ' || rolsuper::text FROM pg_roles WHERE rolname = current_user;")" \
    || fail "could not query the current role's privileges as '${DB_APP_USER}'"

case "$role_flags" in
    "false false") : ;;
    *) fail "the application role '${DB_APP_USER}' holds BYPASSRLS or SUPERUSER (rolbypassrls rolsuper = '${role_flags}'). Row-Level Security would not apply." ;;
esac

# Gate 3 — no persistent connections. The active tenant lives in a PostgreSQL
# session variable, so a connection reused across requests leaks one tenant's
# variable into another's request.
case "${DATABASE_URL:-}" in
    *persistent=true*|*persistent=1*)
        fail "DATABASE_URL requests a persistent connection. The tenant session variable would leak across requests."
        ;;
esac

# Gate 4 — every table declared trainer-scoped must actually have RLS enabled
# and at least one policy. Skipped on a database that has not been migrated yet,
# so that 'docker compose up' works before 'make migrate'.
SCOPED_MANIFEST=/app/config/tenancy/trainer_scoped_tables.txt
GLOBAL_MANIFEST=/app/config/tenancy/resolver_global_tables.txt
migrated="$(psql_app "SELECT to_regclass('public.doctrine_migration_versions') IS NOT NULL;")" || migrated=f

if [ "$migrated" = "t" ] && [ -f "$SCOPED_MANIFEST" ]; then
    missing=""
    while IFS= read -r table; do
        case "$table" in ''|\#*) continue ;; esac
        exists="$(psql_app "SELECT to_regclass('public.${table}') IS NOT NULL;")"
        [ "$exists" = "t" ] || continue
        state="$(psql_app "SELECT c.relrowsecurity::text || ' ' || (SELECT count(*) FROM pg_policy p WHERE p.polrelid = c.oid)::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relname = '${table}';")"
        case "$state" in
            "true 0") missing="${missing} ${table}(no-policy)" ;;
            "false "*) missing="${missing} ${table}(rls-off)" ;;
        esac
    done < "$SCOPED_MANIFEST"

    if [ -n "$missing" ]; then
        fail "Row-Level Security is not in force on trainer-scoped tables:${missing}"
    fi
    log "startup gate passed: RLS in force on all declared trainer-scoped tables"

    # Gate 5 — the mirror image. The tenant resolver reads these tables BEFORE a
    # tenant exists, so a policy on them would be evaluated with no tenant set
    # and return nothing: every login and every public code-resolved route would
    # break, without raising anything. Absence of RLS here is as load-bearing as
    # its presence above.
    if [ -f "$GLOBAL_MANIFEST" ]; then
        wrongly_scoped=""
        while IFS= read -r table; do
            case "$table" in ''|\#*) continue ;; esac
            exists="$(psql_app "SELECT to_regclass('public.${table}') IS NOT NULL;")"
            [ "$exists" = "t" ] || continue
            enabled="$(psql_app "SELECT relrowsecurity::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relname = '${table}';")"
            [ "$enabled" = "true" ] && wrongly_scoped="${wrongly_scoped} ${table}"
        done < "$GLOBAL_MANIFEST"

        if [ -n "$wrongly_scoped" ]; then
            fail "Row-Level Security is enabled on tables the tenant resolver must read before a tenant exists:${wrongly_scoped}. Every login and every public route would silently resolve nothing."
        fi
        log "startup gate passed: no RLS on the resolver's own global tables"
    fi
else
    log "startup gate: schema not migrated yet, RLS table check deferred"
fi

unset PGPASSWORD
log "startup gate passed as role '${DB_APP_USER}'"

# --- Dispatch ----------------------------------------------------------------
case "${1:-php-fpm}" in
    worker)
        # The transport table is created by a migration, not by auto_setup,
        # because the application role holds no DDL. Wait for it rather than
        # crash-looping until someone runs `make migrate`.
        export PGPASSWORD="$DB_APP_PASSWORD"
        attempt=0
        while [ "$(psql_app "SELECT to_regclass('public.messenger_messages') IS NOT NULL;")" != "t" ]; do
            attempt=$((attempt + 1))
            if [ "$attempt" = 1 ]; then
                log "waiting for messenger_messages: run 'make migrate' to create it"
            fi
            sleep 5
        done
        unset PGPASSWORD

        log "starting messenger worker"
        # Symfony Scheduler joins this command as `scheduler_default` once the
        # first Schedule is defined; until then consuming it would fail because
        # the transport does not exist.
        exec su-exec "${APP_UID}:${APP_GID}" php /app/bin/console messenger:consume async \
            --time-limit=3600 --memory-limit=256M -v
        ;;
    php-fpm)
        exec php-fpm --nodaemonize
        ;;
    *)
        exec "$@"
        ;;
esac
