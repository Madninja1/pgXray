#!/usr/bin/env bash
set -euo pipefail

# Opt-in test only: creates and drops thousands of physical user tables.
# Never point this at production, even if the database happens to have the expected name.
: "${PGXRAY_TEST_HOST:?PGXRAY_TEST_HOST must be set to localhost}"
: "${PGXRAY_TEST_USER:?PGXRAY_TEST_USER must be set}"
: "${PGXRAY_TEST_PASSWORD:?PGXRAY_TEST_PASSWORD must be set}"
: "${PGXRAY_TEST_DB:?PGXRAY_TEST_DB must be set}"
[[ ${PGXRAY_TEST_DB} == pgxray_policy_test ]] || {
  echo 'Refusing load test: requires dedicated database pgxray_policy_test' >&2
  exit 2
}
[[ ${PGXRAY_TEST_HOST} == localhost || ${PGXRAY_TEST_HOST} == 127.0.0.1 || ${PGXRAY_TEST_HOST} == ::1 ]] || {
  echo 'Refusing load test on a non-loopback PostgreSQL host' >&2
  exit 2
}
[[ ${PGXRAY_TEST_ALLOW_LOAD:-} == YES_DISPOSABLE_DB ]] || {
  echo 'Refusing load test: set PGXRAY_TEST_ALLOW_LOAD=YES_DISPOSABLE_DB explicitly' >&2
  exit 2
}
command -v psql >/dev/null || { echo 'psql is required' >&2; exit 2; }
command -v cargo >/dev/null || { echo 'cargo is required' >&2; exit 2; }

count=${PGXRAY_TEST_LOAD_TABLES:-1000}
[[ $count =~ ^[0-9]+$ ]] && (( count >= 1000 && count <= 5000 )) || {
  echo 'PGXRAY_TEST_LOAD_TABLES must be an integer from 1000 to 5000' >&2
  exit 2
}

export PGHOST=$PGXRAY_TEST_HOST
export PGPORT=${PGXRAY_TEST_PORT:-5432}
export PGDATABASE=$PGXRAY_TEST_DB
export PGUSER=$PGXRAY_TEST_USER
export PGPASSWORD=$PGXRAY_TEST_PASSWORD
prefix="pgxray_load_${BASHPID}_${RANDOM}_"

cleanup() {
  echo "Removing load-test fixture tables ($prefix)..."
  psql -X -v ON_ERROR_STOP=1 -v prefix="$prefix" -v count="$count" <<'SQL'
SELECT set_config('pgxray.load_prefix', :'prefix', false);
SELECT set_config('pgxray.load_count', :'count', false);
DO $pgxray$
DECLARE i integer;
BEGIN
    FOR i IN 1..current_setting('pgxray.load_count')::integer LOOP
        EXECUTE format('DROP TABLE IF EXISTS public.%I', current_setting('pgxray.load_prefix') || i);
    END LOOP;
END
$pgxray$;
SQL
}
trap cleanup EXIT

printf 'Creating %s disposable tables (prefix %s)...\n' "$count" "$prefix"
psql -X -v ON_ERROR_STOP=1 -v prefix="$prefix" -v count="$count" <<'SQL'
SELECT set_config('pgxray.load_prefix', :'prefix', false);
SELECT set_config('pgxray.load_count', :'count', false);
SET statement_timeout = '180s';
DO $pgxray$
DECLARE i integer;
BEGIN
    FOR i IN 1..current_setting('pgxray.load_count')::integer LOOP
        EXECUTE format('CREATE TABLE public.%I AS SELECT generate_series(1, 10) AS id', current_setting('pgxray.load_prefix') || i);
    END LOOP;
END
$pgxray$;
SQL

actual=$(psql -X -A -t -v ON_ERROR_STOP=1 -v prefix="$prefix" <<'SQL'
SELECT count(*) FROM pg_class AS c
JOIN pg_namespace AS n ON n.oid = c.relnamespace
WHERE n.nspname = 'public' AND c.relkind = 'r'
  AND left(c.relname, length(:'prefix')) = :'prefix';
SQL
)
[[ "$actual" == "$count" ]] || {
  echo "Expected $count fixture tables, got $actual" >&2
  exit 1
}

echo "Created $actual tables. Timing real collector and single-table detail (10s SQL / 32 MiB output policy)."
time cargo test -p pgxray-core --lib \
  autovacuum::tests::integration_autovacuum_collect_and_table_detail_readonly \
  -- --ignored --nocapture
