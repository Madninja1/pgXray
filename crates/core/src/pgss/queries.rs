pub(super) const OVERVIEW_SQL: &str = r#"
WITH extension_state AS (
    SELECT
        EXISTS (
            SELECT 1
            FROM pg_extension
            WHERE extname = 'pg_stat_statements'
        ) AS installed,
        (
            SELECT extversion
            FROM pg_extension
            WHERE extname = 'pg_stat_statements'
        ) AS extension_version
),
preload_state AS (
    SELECT EXISTS (
        SELECT 1
        FROM regexp_split_to_table(
            current_setting('shared_preload_libraries'),
            ','
        ) AS value
        WHERE replace(replace(btrim(value), chr(34), ''), chr(39), '')
            IN ('pg_stat_statements', '$libdir/pg_stat_statements')
    ) AS preloaded
)
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', d.oid::bigint,
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'extension_installed', e.installed,
    'extension_version', e.extension_version,
    'preloaded', p.preloaded,
    'shared_preload_libraries',
        current_setting('shared_preload_libraries'),
    'compute_query_id',
        current_setting('compute_query_id', true),
    'max_entries',
        CASE
            WHEN current_setting('pg_stat_statements.max', true) IS NULL
            THEN NULL
            ELSE current_setting('pg_stat_statements.max', true)::bigint
        END,
    'track',
        current_setting('pg_stat_statements.track', true),
    'track_utility',
        CASE current_setting('pg_stat_statements.track_utility', true)
            WHEN 'on' THEN true
            WHEN 'off' THEN false
            ELSE NULL
        END,
    'track_planning',
        CASE current_setting('pg_stat_statements.track_planning', true)
            WHEN 'on' THEN true
            WHEN 'off' THEN false
            ELSE NULL
        END,
    'save',
        CASE current_setting('pg_stat_statements.save', true)
            WHEN 'on' THEN true
            WHEN 'off' THEN false
            ELSE NULL
        END,
    'track_io_timing',
        current_setting('track_io_timing') = 'on'
)::text
FROM pg_database AS d
CROSS JOIN extension_state AS e
CROSS JOIN preload_state AS p
WHERE d.datname = current_database();
"#;

pub(super) const STATS_SQL: &str = r#"
SELECT json_build_object(
    'dealloc', i.dealloc,
    'stats_reset', i.stats_reset,
    'stats_age_seconds',
        CASE WHEN i.stats_reset IS NULL THEN NULL
        ELSE GREATEST(
            0,
            extract(
                epoch FROM clock_timestamp() - i.stats_reset
            )::bigint
        ) END
)::text
FROM pg_stat_statements_info AS i;
"#;

pub(super) const CAPACITY_SQL: &str = r#"
WITH counts AS (
    SELECT
        count(*)::bigint AS current_entries,
        count(*) FILTER (WHERE calls = 1)::bigint
            AS one_call_entries,
        count(*) FILTER (WHERE calls <= 5)::bigint
            AS up_to_5_calls_entries,
        COALESCE(sum(calls), 0)::bigint AS total_calls,
        COALESCE(sum(total_exec_time), 0)::double precision
            AS total_exec_time_ms
    FROM pg_stat_statements(false)
),
cfg AS (
    SELECT
        current_setting('pg_stat_statements.max')::bigint
            AS max_entries
)
SELECT json_build_object(
    'max_entries', cfg.max_entries,
    'current_entries', c.current_entries,
    'fill_pct',
        round(
            c.current_entries::numeric
            / NULLIF(cfg.max_entries::numeric, 0)
            * 100,
            1
        )::double precision,
    'one_call_entries', c.one_call_entries,
    'one_call_pct',
        COALESCE(
            round(
                c.one_call_entries::numeric
                / NULLIF(c.current_entries::numeric, 0)
                * 100,
                1
            ),
            0
        )::double precision,
    'up_to_5_calls_entries', c.up_to_5_calls_entries,
    'up_to_5_calls_pct',
        COALESCE(
            round(
                c.up_to_5_calls_entries::numeric
                / NULLIF(c.current_entries::numeric, 0)
                * 100,
                1
            ),
            0
        )::double precision,
    'total_calls', c.total_calls,
    'total_exec_time_ms', c.total_exec_time_ms
)::text
FROM counts AS c
CROSS JOIN cfg;
"#;

pub(super) const ROLE_DATABASE_SQL: &str = r#"
WITH base AS (
    SELECT
        s.userid,
        s.dbid,
        s.calls
    FROM pg_stat_statements(false) AS s
),
totals AS (
    SELECT count(*)::numeric AS entries
    FROM base
),
groups AS (
    SELECT
        b.userid,
        b.dbid,
        count(*)::bigint AS entries,
        count(*) FILTER (WHERE b.calls = 1)::bigint
            AS one_call_entries,
        count(*) FILTER (WHERE b.calls <= 5)::bigint
            AS up_to_5_calls_entries,
        COALESCE(sum(b.calls), 0)::bigint AS total_calls
    FROM base AS b
    GROUP BY
        b.userid,
        b.dbid
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        g.userid::bigint AS userid,
        g.dbid::bigint AS dbid,
        r.rolname::text AS role_name,
        d.datname::text AS database,
        g.entries,
        round(
            g.entries::numeric
            / NULLIF(t.entries, 0)
            * 100,
            1
        )::double precision AS entries_pct,
        g.one_call_entries,
        round(
            g.one_call_entries::numeric
            / NULLIF(g.entries::numeric, 0)
            * 100,
            1
        )::double precision AS one_call_pct,
        g.up_to_5_calls_entries,
        round(
            g.up_to_5_calls_entries::numeric
            / NULLIF(g.entries::numeric, 0)
            * 100,
            1
        )::double precision AS up_to_5_calls_pct,
        g.total_calls
    FROM groups AS g
    CROSS JOIN totals AS t
    LEFT JOIN pg_roles AS r
        ON r.oid = g.userid
    LEFT JOIN pg_database AS d
        ON d.oid = g.dbid
    ORDER BY
        g.entries DESC,
        g.one_call_entries DESC
    LIMIT 100
) AS q;
"#;

pub(super) const COMMANDS_SQL: &str = r#"
WITH base AS (
    SELECT
        s.userid,
        s.dbid,
        s.calls,
        CASE
            WHEN s.query IS NULL THEN '<HIDDEN>'
            -- The leading WITH does not identify the statement's DML command.
            -- Do not guess SELECT/INSERT/UPDATE/DELETE without parsing SQL.
            WHEN s.query ~* '^[[:space:]]*WITH([[:space:]]|$)'
                THEN '<WITH/CTE>'
            ELSE upper(
                COALESCE(
                    (regexp_match(s.query, '^[[:space:]]*([[:alpha:]]+)'))[1],
                    '<OTHER>'
                )
            )
        END AS command
    FROM pg_stat_statements AS s
),
totals AS (
    SELECT count(*)::numeric AS entries
    FROM base
),
groups AS (
    SELECT
        b.userid,
        b.dbid,
        b.command,
        count(*)::bigint AS entries,
        count(*) FILTER (WHERE b.calls = 1)::bigint
            AS one_call_entries,
        count(*) FILTER (WHERE b.calls <= 5)::bigint
            AS up_to_5_calls_entries,
        COALESCE(sum(b.calls), 0)::bigint AS total_calls
    FROM base AS b
    GROUP BY
        b.userid,
        b.dbid,
        b.command
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        g.userid::bigint AS userid,
        g.dbid::bigint AS dbid,
        r.rolname::text AS role_name,
        d.datname::text AS database,
        g.command::text AS command,
        g.entries,
        round(
            g.entries::numeric
            / NULLIF(t.entries, 0)
            * 100,
            1
        )::double precision AS entries_pct,
        g.one_call_entries,
        round(
            g.one_call_entries::numeric
            / NULLIF(g.entries::numeric, 0)
            * 100,
            1
        )::double precision AS one_call_pct,
        g.up_to_5_calls_entries,
        round(
            g.up_to_5_calls_entries::numeric
            / NULLIF(g.entries::numeric, 0)
            * 100,
            1
        )::double precision AS up_to_5_calls_pct,
        g.total_calls
    FROM groups AS g
    CROSS JOIN totals AS t
    LEFT JOIN pg_roles AS r
        ON r.oid = g.userid
    LEFT JOIN pg_database AS d
        ON d.oid = g.dbid
    ORDER BY
        g.entries DESC,
        g.one_call_entries DESC
    LIMIT 150
) AS q;
"#;

pub(super) const CALL_TARGETS_SQL: &str = r#"
WITH cfg AS (
    SELECT
        current_setting('pg_stat_statements.max')::numeric
            AS max_entries
),
calls AS (
    SELECT
        s.userid,
        s.dbid,
        md5(
            btrim(
                split_part(
                    regexp_replace(
                        s.query,
                        '^[[:space:]]*CALL[[:space:]]+',
                        '',
                        'i'
                    ),
                    '(',
                    1
                )
            )
        ) AS call_tag,
        left(
            btrim(split_part(regexp_replace(s.query,
                '^[[:space:]]*CALL[[:space:]]+', '', 'i'), '(', 1)),
            160
        ) AS call_target_name,
        s.calls
    FROM pg_stat_statements AS s
    WHERE s.query IS NOT NULL
      AND s.query ~* '^[[:space:]]*CALL[[:space:]]+'
),
groups AS (
    SELECT
        c.userid,
        c.dbid,
        c.call_tag,
        max(c.call_target_name)::text AS call_target_name,
        count(*)::bigint AS entries,
        count(*) FILTER (WHERE c.calls = 1)::bigint
            AS one_call_entries,
        COALESCE(sum(c.calls), 0)::bigint AS total_calls
    FROM calls AS c
    GROUP BY
        c.userid,
        c.dbid,
        c.call_tag
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        g.userid::bigint AS userid,
        g.dbid::bigint AS dbid,
        r.rolname::text AS role_name,
        d.datname::text AS database,
        g.call_tag,
        g.call_target_name,
        g.entries,
        round(
            g.entries::numeric
            / NULLIF(cfg.max_entries, 0)
            * 100,
            1
        )::double precision AS entries_pct_of_max,
        g.one_call_entries,
        round(
            g.one_call_entries::numeric
            / NULLIF(g.entries::numeric, 0)
            * 100,
            1
        )::double precision AS one_call_pct,
        g.total_calls
    FROM groups AS g
    CROSS JOIN cfg
    LEFT JOIN pg_roles AS r
        ON r.oid = g.userid
    LEFT JOIN pg_database AS d
        ON d.oid = g.dbid
    ORDER BY
        g.entries DESC,
        g.one_call_entries DESC
    LIMIT 50
) AS q;
"#;

pub(super) const PERFORMANCE_SQL: &str = r#"
WITH base AS MATERIALIZED (
    SELECT
        s.userid::bigint AS userid,
        s.dbid::bigint AS dbid,
        r.rolname::text AS role_name,
        d.datname::text AS database,
        s.toplevel,
        s.queryid::text AS queryid,
        s.calls,
        s.plans,
        s.total_plan_time AS total_plan_time_ms,
        s.mean_plan_time AS mean_plan_time_ms,
        s.stddev_plan_time AS stddev_plan_time_ms,
        s.total_exec_time AS total_exec_time_ms,
        s.min_exec_time AS min_exec_time_ms,
        s.max_exec_time AS max_exec_time_ms,
        s.mean_exec_time AS mean_exec_time_ms,
        s.stddev_exec_time AS stddev_exec_time_ms,
        s.rows,
        s.shared_blks_hit,
        s.shared_blks_read,
        s.shared_blks_dirtied,
        s.shared_blks_written,
        s.temp_blks_read,
        s.temp_blks_written,
        s.blk_read_time AS blk_read_time_ms,
        s.blk_write_time AS blk_write_time_ms,
        s.wal_records,
        s.wal_fpi,
        s.wal_bytes::double precision AS wal_bytes
    FROM pg_stat_statements(false) AS s
    LEFT JOIN pg_roles AS r
        ON r.oid = s.userid
    LEFT JOIN pg_database AS d
        ON d.oid = s.dbid
),
-- Bounded top-N sorts avoid eight full-table window rankings.
-- Each ranking processes only 50 finalists. Entries may occur in more than one TOP.
history_candidates AS MATERIALIZED (
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY total_exec_time_ms DESC, calls DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, total_exec_time_ms, calls
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY total_exec_time_ms DESC, calls DESC
            LIMIT 50
        ) AS top_1
    ) AS ranked_1
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY calls DESC, total_exec_time_ms DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, calls, total_exec_time_ms
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY calls DESC, total_exec_time_ms DESC
            LIMIT 50
        ) AS top_2
    ) AS ranked_2
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY mean_exec_time_ms DESC, calls DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, mean_exec_time_ms, calls
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY mean_exec_time_ms DESC, calls DESC
            LIMIT 50
        ) AS top_3
    ) AS ranked_3
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY (temp_blks_written + temp_blks_read) DESC, total_exec_time_ms DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, temp_blks_written, temp_blks_read, total_exec_time_ms
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY (temp_blks_written + temp_blks_read) DESC, total_exec_time_ms DESC
            LIMIT 50
        ) AS top_4
    ) AS ranked_4
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY shared_blks_read DESC, total_exec_time_ms DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, shared_blks_read, total_exec_time_ms
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY shared_blks_read DESC, total_exec_time_ms DESC
            LIMIT 50
        ) AS top_5
    ) AS ranked_5
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY wal_bytes DESC, calls DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, wal_bytes, calls
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY wal_bytes DESC, calls DESC
            LIMIT 50
        ) AS top_6
    ) AS ranked_6
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY stddev_exec_time_ms DESC, max_exec_time_ms DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, stddev_exec_time_ms, max_exec_time_ms
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY stddev_exec_time_ms DESC, max_exec_time_ms DESC
            LIMIT 50
        ) AS top_7
    ) AS ranked_7
    UNION ALL
    SELECT userid, dbid, queryid, toplevel, history_rank
    FROM (
        SELECT userid, dbid, queryid, toplevel,
               row_number() OVER (ORDER BY total_plan_time_ms DESC, plans DESC) AS history_rank
        FROM (
            SELECT userid, dbid, queryid, toplevel, total_plan_time_ms, plans
            FROM base
            WHERE queryid IS NOT NULL
            ORDER BY total_plan_time_ms DESC, plans DESC
            LIMIT 50
        ) AS top_8
    ) AS ranked_8
),
history_ranked AS MATERIALIZED (
    SELECT DISTINCT ON (b.userid, b.dbid, b.queryid, b.toplevel)
        b.*, c.history_rank
    FROM history_candidates AS c
    JOIN base AS b USING (userid, dbid, queryid, toplevel)
    ORDER BY b.userid, b.dbid, b.queryid, b.toplevel, c.history_rank
)
SELECT json_build_object(
    'total_exec',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    ORDER BY
                        total_exec_time_ms DESC,
                        calls DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'mean_exec',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    ORDER BY
                        mean_exec_time_ms DESC,
                        calls DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'temp_io',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    WHERE temp_blks_written > 0
                       OR temp_blks_read > 0
                    ORDER BY
                        temp_blks_written DESC,
                        temp_blks_read DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'shared_reads',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    WHERE shared_blks_read > 0
                    ORDER BY
                        shared_blks_read DESC,
                        total_exec_time_ms DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'wal',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    WHERE wal_bytes > 0
                    ORDER BY
                        wal_bytes DESC,
                        calls DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'variance',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    WHERE calls >= 2
                    ORDER BY
                        stddev_exec_time_ms DESC,
                        max_exec_time_ms DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'planning',
        COALESCE(
            (
                SELECT json_agg(row_to_json(x))
                FROM (
                    SELECT *
                    FROM base
                    WHERE total_plan_time_ms > 0
                    ORDER BY
                        total_plan_time_ms DESC,
                        plans DESC
                    LIMIT 50
                ) AS x
            ),
            '[]'::json
        ),
    'history_sample',
        COALESCE(
            (
                SELECT json_agg(to_jsonb(x) - 'history_rank')
                FROM (
                    SELECT *
                    FROM history_ranked
                    WHERE history_rank <= 50
                    ORDER BY
                        history_rank,
                        total_exec_time_ms DESC,
                        calls DESC
                    LIMIT 300
                ) AS x
            ),
            '[]'::json
        )
)::text;
"#;
