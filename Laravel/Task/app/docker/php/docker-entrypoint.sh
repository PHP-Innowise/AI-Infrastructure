#!/bin/sh
# PracticePerfect container entrypoint and startup gate, Laravel edition.
#
# Row-Level Security has one failure mode it cannot detect itself: if the
# application connects as the table owner, or as a role holding BYPASSRLS,
# every policy is skipped and nothing complains. This gate refuses to start the
# container rather than serve one trainer's rows to another.
#
# See specs/architect-architecture.md, "Tenancy enforcement".
set -eu

APP_UID="${APP_UID:-1000}"
APP_GID="${APP_GID:-1000}"

DB_HOST="${DB_HOST:-db}"
DB_PORT="${DB_PORT:-5432}"
DB_NAME="${DB_DATABASE:-practiceperfect}"
DB_OWNER="${DB_OWNER_USERNAME:-pp_owner}"
DB_APP_USER="${DB_USERNAME:-pp_app}"
DB_APP_PASSWORD="${DB_PASSWORD:-}"

log()  { printf '[entrypoint] %s\n' "$*" >&2; }
fail() { printf '[entrypoint] FATAL: %s\n' "$*" >&2; exit 1; }

# --- Writable runtime directories -------------------------------------------
# storage/ is a named volume, and a fresh one mounts empty — it shadows the
# skeleton's own tree. Laravel does not create these on demand: a missing
# storage/framework/views fails every request with "Please provide a valid
# cache path", which reads like a configuration error rather than a missing
# directory. Creating them here makes the container correct on first boot and
# after anyone empties the volume.
for dir in \
    /app/storage/app/public \
    /app/storage/framework/cache/data \
    /app/storage/framework/sessions \
    /app/storage/framework/testing \
    /app/storage/framework/views \
    /app/storage/logs \
    /app/bootstrap/cache; do
    mkdir -p "$dir" 2>/dev/null || true
    chown -R "${APP_UID}:${APP_GID}" "$dir" 2>/dev/null || true
done

# Compiled Vite assets from the build stage, when the bind mount has none.
if [ -d /opt/assets/build ] && [ ! -d /app/public/build ]; then
    cp -r /opt/assets/build /app/public/build 2>/dev/null || true
    chown -R "${APP_UID}:${APP_GID}" /app/public/build 2>/dev/null || true
fi

# --- Wait for PostgreSQL -----------------------------------------------------
log "waiting for postgres at ${DB_HOST}:${DB_PORT}"
attempt=0
until pg_isready -h "$DB_HOST" -p "$DB_PORT" -U "$DB_APP_USER" -d "$DB_NAME" >/dev/null 2>&1; do
    attempt=$((attempt + 1))
    [ "$attempt" -ge 60 ] && fail "postgres did not become ready within 60 attempts"
    sleep 1
done
log "postgres is ready"

export PGPASSWORD="$DB_APP_PASSWORD"
psql_app() {
    psql -h "$DB_HOST" -p "$DB_PORT" -U "$DB_APP_USER" -d "$DB_NAME" \
         -t -A -v ON_ERROR_STOP=1 -c "$1"
}

# Gate 1 — the application role must not own the schema. An owner is exempt
# from its own policies unless FORCE ROW LEVEL SECURITY is set.
if [ "$DB_APP_USER" = "$DB_OWNER" ]; then
    fail "the application is configured to connect as the schema owner '${DB_OWNER}'. Row-Level Security would not apply. Set DB_USERNAME to the unprivileged application role."
fi

# Gate 2 — the application role must not hold BYPASSRLS or superuser.
role_flags="$(psql_app "SELECT rolbypassrls::text || ' ' || rolsuper::text FROM pg_roles WHERE rolname = current_user;")" \
    || fail "could not query the current role's privileges as '${DB_APP_USER}'"

case "$role_flags" in
    "false false") : ;;
    *) fail "the application role '${DB_APP_USER}' holds BYPASSRLS or SUPERUSER (rolbypassrls rolsuper = '${role_flags}'). Row-Level Security would not apply." ;;
esac

# Gate 3 — RLS is in force on every declared trainer-scoped table, and absent
# from the resolver's own global tables. Both directions matter: the resolver
# reads its tables before a tenant exists, so a policy there would make every
# login silently resolve nothing.
SCOPED_MANIFEST=/app/config/tenancy/trainer_scoped_tables.txt
GLOBAL_MANIFEST=/app/config/tenancy/resolver_global_tables.txt

migrated="$(psql_app "SELECT to_regclass('public.migrations') IS NOT NULL;")" || migrated=f

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

    [ -n "$missing" ] && fail "Row-Level Security is not in force on trainer-scoped tables:${missing}"
    log "startup gate passed: RLS in force on all declared trainer-scoped tables"

    if [ -f "$GLOBAL_MANIFEST" ]; then
        wrongly_scoped=""
        while IFS= read -r table; do
            case "$table" in ''|\#*) continue ;; esac
            exists="$(psql_app "SELECT to_regclass('public.${table}') IS NOT NULL;")"
            [ "$exists" = "t" ] || continue
            enabled="$(psql_app "SELECT relrowsecurity::text FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = 'public' AND c.relname = '${table}';")"
            [ "$enabled" = "true" ] && wrongly_scoped="${wrongly_scoped} ${table}"
        done < "$GLOBAL_MANIFEST"

        [ -n "$wrongly_scoped" ] && fail "Row-Level Security is enabled on tables the tenant resolver must read before a tenant exists:${wrongly_scoped}. Every login and every public route would silently resolve nothing."
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
        # The jobs table is created by a migration, not on demand. Wait for it
        # rather than crash-looping until someone runs `make migrate`.
        export PGPASSWORD="$DB_APP_PASSWORD"
        attempt=0
        while [ "$(psql_app "SELECT to_regclass('public.jobs') IS NOT NULL;")" != "t" ]; do
            attempt=$((attempt + 1))
            [ "$attempt" = 1 ] && log "waiting for the jobs table: run 'make migrate' to create it"
            sleep 5
        done
        unset PGPASSWORD

        log "starting queue worker"
        exec su-exec "${APP_UID}:${APP_GID}" php /app/artisan queue:work database \
            --tries=3 --max-time=3600 --sleep=1 -v
        ;;
    php-fpm)
        exec php-fpm --nodaemonize
        ;;
    *)
        exec "$@"
        ;;
esac
