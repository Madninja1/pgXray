pub(super) const OVERVIEW_SQL: &str = r#"
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', d.oid::bigint,
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'postmaster_start_time', EXTRACT(EPOCH FROM pg_postmaster_start_time())::numeric::text,
    'in_recovery', pg_is_in_recovery(),
    -- Snapshot upper bound is a read-only XID position, not this session's assigned XID.
    'current_xid', pg_snapshot_xmax(pg_current_snapshot())::text,
    'autovacuum', current_setting('autovacuum') = 'on',
    'autovacuum_freeze_max_age',
        current_setting('autovacuum_freeze_max_age')::bigint,
    'vacuum_freeze_table_age',
        current_setting('vacuum_freeze_table_age')::bigint,
    'vacuum_freeze_min_age',
        current_setting('vacuum_freeze_min_age')::bigint,
    'autovacuum_max_workers',
        current_setting('autovacuum_max_workers')::integer,
    'autovacuum_vacuum_cost_limit',
        current_setting('autovacuum_vacuum_cost_limit')::integer,
    'autovacuum_vacuum_cost_delay',
        current_setting('autovacuum_vacuum_cost_delay'),
    'database_xid_age',
        age(d.datfrozenxid)::bigint,
    'stats_reset',
        s.stats_reset,
    'database_mxid_age', mxid_age(d.datminmxid)::bigint,
    'autovacuum_multixact_freeze_max_age',
        current_setting('autovacuum_multixact_freeze_max_age')::bigint
)::text
FROM pg_database AS d
LEFT JOIN pg_stat_database AS s
    ON s.datid = d.oid
WHERE d.datname = current_database();
"#;

pub(super) const DATABASES_SQL: &str = r#"
WITH cfg AS (
    SELECT
        current_setting('autovacuum_freeze_max_age')::numeric AS max_age,
        current_setting('autovacuum_multixact_freeze_max_age')::numeric AS max_mxid_age
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        d.oid::bigint AS database_oid,
        d.datname::text AS database,
        age(d.datfrozenxid)::bigint AS xid_age,
        mxid_age(d.datminmxid)::bigint AS mxid_age,
        round(
            mxid_age(d.datminmxid)::numeric
            / NULLIF(cfg.max_mxid_age, 0) * 100, 1
        )::double precision AS pct_of_global_multixact_freeze_max,
        round(
            age(d.datfrozenxid)::numeric
            / NULLIF(cfg.max_age, 0)
            * 100,
            1
        )::double precision AS pct_of_global_freeze_max
    FROM pg_database AS d
    CROSS JOIN cfg
    WHERE d.datallowconn
    ORDER BY age(d.datfrozenxid) DESC
) AS q;
"#;

pub(super) const RELATIONS_SQL: &str = r#"
WITH global_cfg AS (
    SELECT
        current_setting('autovacuum_freeze_max_age')::bigint AS freeze_max_age,
        current_setting('vacuum_freeze_min_age')::bigint AS freeze_min_age,
        current_setting('vacuum_freeze_table_age')::bigint AS freeze_table_age,
        current_setting('autovacuum_multixact_freeze_max_age')::bigint AS multixact_max_age
),
relations AS (
    SELECT
        c.oid,
        c.relnamespace,
        c.relname,
        c.relkind,
        c.relfrozenxid,
        c.relminmxid,
        c.reltuples,
        c.relpages,
        c.relallvisible,
        -- relallfrozen was introduced in PG18; do not resolve a missing column on PG16/17.
        (to_jsonb(c)->>'relallfrozen')::bigint AS relallfrozen,
        c.reloptions
    FROM pg_class AS c
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'm', 't')
      AND n.nspname !~ '^pg_temp_'
      AND n.nspname !~ '^pg_toast_temp_'
    /* pgxray:xid-page-relations */
),
effective AS (
    SELECT
        r.*,
        LEAST(COALESCE(opts.table_freeze_max_age, cfg.freeze_max_age),
              cfg.freeze_max_age)::bigint AS effective_freeze_max_age,
        COALESCE(opts.table_multixact_max_age, cfg.multixact_max_age)::bigint
            AS effective_multixact_freeze_max_age,
        CASE
            WHEN opts.table_freeze_max_age IS NULL
                THEN 'global'
            ELSE 'table'
        END AS freeze_max_source,
        LEAST(COALESCE(opts.table_freeze_min_age, cfg.freeze_min_age),
              cfg.freeze_max_age / 2)::bigint AS effective_freeze_min_age,
        CASE
            WHEN opts.table_freeze_min_age IS NULL
                THEN 'global'
            ELSE 'table'
        END AS freeze_min_source,
        LEAST(COALESCE(opts.table_freeze_table_age, cfg.freeze_table_age),
              floor(0.95 * LEAST(COALESCE(opts.table_freeze_max_age, cfg.freeze_max_age),
                                  cfg.freeze_max_age)))::bigint AS effective_freeze_table_age,
        CASE
            WHEN opts.table_freeze_table_age IS NULL
                THEN 'global'
            ELSE 'table'
        END AS freeze_table_source
    FROM relations AS r
    CROSS JOIN global_cfg AS cfg
    LEFT JOIN LATERAL (
        SELECT
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_freeze_max_age'
                    THEN o.option_value::bigint
                END
            ) AS table_freeze_max_age,
            max(o.option_value::bigint) FILTER
                (WHERE o.option_name = 'autovacuum_multixact_freeze_max_age')
                AS table_multixact_max_age,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_freeze_min_age'
                    THEN o.option_value::bigint
                END
            ) AS table_freeze_min_age,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_freeze_table_age'
                    THEN o.option_value::bigint
                END
            ) AS table_freeze_table_age
        FROM pg_options_to_table(r.reloptions) AS o
    ) AS opts
        ON true
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        e.oid::bigint AS relation_oid,
        n.nspname::text AS schema_name,
        e.relname::text AS relation_name,
        e.relkind::text AS relation_type,
        age(e.relfrozenxid)::bigint AS xid_age,
        mxid_age(e.relminmxid)::bigint AS mxid_age,
        e.effective_multixact_freeze_max_age,
        round(mxid_age(e.relminmxid)::numeric
              / NULLIF(e.effective_multixact_freeze_max_age::numeric, 0) * 100, 1
        )::double precision AS pct_of_multixact_freeze_max,
        e.effective_freeze_max_age,
        e.freeze_max_source,
        round(
            age(e.relfrozenxid)::numeric
            / NULLIF(e.effective_freeze_max_age::numeric, 0)
            * 100,
            1
        )::double precision AS pct_of_freeze_max,
        pg_total_relation_size(e.oid)::bigint AS total_size_bytes,
        CASE WHEN e.reltuples >= 0 THEN round(e.reltuples)::bigint END AS estimated_rows,
        GREATEST(e.relpages::bigint, 0) AS heap_pages,
        GREATEST(e.relallvisible::bigint, 0) AS all_visible_pages,
        e.relallfrozen AS all_frozen_pages,
        CASE
            WHEN e.relpages > 0
            THEN round(
                e.relallvisible::numeric
                / e.relpages::numeric
                * 100,
                1
            )::double precision
        END AS all_visible_pct,
        e.effective_freeze_min_age,
        e.freeze_min_source,
        e.effective_freeze_table_age,
        e.freeze_table_source,
        s.last_vacuum,
        s.last_autovacuum,
        s.last_analyze,
        s.last_autoanalyze,
        COALESCE(s.vacuum_count, 0)::bigint AS vacuum_count,
        COALESCE(s.autovacuum_count, 0)::bigint AS autovacuum_count,
        COALESCE(s.analyze_count, 0)::bigint AS analyze_count,
        COALESCE(s.autoanalyze_count, 0)::bigint AS autoanalyze_count,
        COALESCE(s.n_mod_since_analyze, 0)::bigint AS modifications_since_analyze,
        e.reloptions,
        parent.oid::bigint AS parent_oid,
        parent_ns.nspname::text AS parent_schema,
        parent.relname::text AS parent_relation,
        parent.relkind::text AS parent_relation_type,
        CASE
            WHEN parent.oid IS NOT NULL
            THEN age(parent.relfrozenxid)::bigint
        END AS parent_xid_age
    FROM effective AS e
    JOIN pg_namespace AS n
        ON n.oid = e.relnamespace
    LEFT JOIN pg_stat_all_tables AS s
        ON s.relid = e.oid
    LEFT JOIN pg_class AS parent
        ON parent.reltoastrelid = e.oid
    LEFT JOIN pg_namespace AS parent_ns
        ON parent_ns.oid = parent.relnamespace
    ORDER BY age(e.relfrozenxid) DESC, e.oid
) AS q;
"#;

pub(super) const AUTOVACUUM_TRIGGERS_SQL: &str = r#"
WITH global_cfg AS (
    SELECT
        current_setting('autovacuum') = 'on'
            AS global_autovacuum_enabled,
        current_setting('autovacuum_vacuum_threshold')::numeric
            AS vacuum_threshold,
        current_setting('autovacuum_vacuum_scale_factor')::numeric
            AS vacuum_scale_factor,
        COALESCE(NULLIF(current_setting('autovacuum_vacuum_max_threshold', true), '')::numeric, -1)
            AS vacuum_max_threshold,
        current_setting('server_version_num')::integer AS server_version_num,
        current_setting('autovacuum_vacuum_insert_threshold')::numeric
            AS insert_threshold,
        current_setting('autovacuum_vacuum_insert_scale_factor')::numeric
            AS insert_scale_factor
),
targets AS (
    SELECT
        c.oid,
        c.relnamespace,
        c.relname,
        c.relkind,
        c.relfrozenxid,
        c.reltuples,
        c.relpages,
        COALESCE((to_jsonb(c)->>'relallfrozen')::bigint, 0) AS relallfrozen,
        c.reloptions
    FROM pg_class AS c
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    WHERE c.relkind IN ('r', 'm', 't')
      AND n.nspname !~ '^pg_temp_'
      AND n.nspname !~ '^pg_toast_temp_'
    /* pgxray:xid-page-triggers */
),
effective AS (
    SELECT
        t.*,
        (
            cfg.global_autovacuum_enabled
            AND COALESCE(
                opts.table_autovacuum_enabled,
                true
            )
        ) AS autovacuum_enabled,
        COALESCE(
            opts.table_vacuum_threshold,
            cfg.vacuum_threshold
        ) AS vacuum_threshold,
        COALESCE(
            opts.table_vacuum_scale_factor,
            cfg.vacuum_scale_factor
        ) AS vacuum_scale_factor,
        COALESCE(opts.table_vacuum_max_threshold, cfg.vacuum_max_threshold)
            AS vacuum_max_threshold,
        cfg.server_version_num,
        COALESCE(
            opts.table_insert_threshold,
            cfg.insert_threshold
        ) AS insert_threshold,
        COALESCE(
            opts.table_insert_scale_factor,
            cfg.insert_scale_factor
        ) AS insert_scale_factor
    FROM targets AS t
    CROSS JOIN global_cfg AS cfg
    LEFT JOIN LATERAL (
        SELECT
            bool_or(o.option_value::boolean)
                FILTER (
                    WHERE o.option_name = 'autovacuum_enabled'
                ) AS table_autovacuum_enabled,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_vacuum_threshold'
                    THEN o.option_value::numeric
                END
            ) AS table_vacuum_threshold,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_vacuum_scale_factor'
                    THEN o.option_value::numeric
                END
            ) AS table_vacuum_scale_factor,
            max(o.option_value::numeric) FILTER
                (WHERE o.option_name = 'autovacuum_vacuum_max_threshold')
                AS table_vacuum_max_threshold,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_vacuum_insert_threshold'
                    THEN o.option_value::numeric
                END
            ) AS table_insert_threshold,
            max(
                CASE
                    WHEN o.option_name = 'autovacuum_vacuum_insert_scale_factor'
                    THEN o.option_value::numeric
                END
            ) AS table_insert_scale_factor
        FROM pg_options_to_table(t.reloptions) AS o
    ) AS opts
        ON true
),
thresholds AS (
    SELECT
        e.*,
        CASE WHEN e.reltuples >= 0 THEN
            floor(CASE WHEN e.vacuum_max_threshold >= 0
                THEN LEAST(e.vacuum_max_threshold,
                           e.vacuum_threshold + e.vacuum_scale_factor * e.reltuples::numeric)
                ELSE e.vacuum_threshold + e.vacuum_scale_factor * e.reltuples::numeric
            END)::bigint + 1
        END AS dead_tuple_trigger,
        CASE WHEN e.insert_threshold >= 0 AND e.reltuples >= 0 THEN
            floor(e.insert_threshold + e.insert_scale_factor * e.reltuples::numeric
                * CASE WHEN e.server_version_num >= 180000 AND e.relpages > 0
                    THEN 1 - LEAST(e.relallfrozen, e.relpages)::numeric / e.relpages
                    ELSE 1 END)::bigint + 1
        END AS insert_trigger
    FROM effective AS e
),
db_stats AS (
    SELECT
        stats_reset
    FROM pg_stat_database
    WHERE datname = current_database()
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        e.oid::bigint AS relation_oid,
        n.nspname::text AS schema_name,
        e.relname::text AS relation_name,
        e.relkind::text AS relation_type,
        age(e.relfrozenxid)::bigint AS xid_age,
        e.autovacuum_enabled,
        CASE WHEN e.reltuples >= 0 THEN round(e.reltuples)::bigint END AS estimated_rows,
        (e.dead_tuple_trigger IS NOT NULL) AS thresholds_reliable,
        COALESCE(s.n_live_tup, 0)::bigint AS estimated_live_rows,
        COALESCE(s.n_dead_tup, 0)::bigint AS estimated_dead_rows,
        COALESCE(s.n_ins_since_vacuum, 0)::bigint AS inserts_since_vacuum,
        COALESCE(s.n_tup_ins, 0)::bigint AS tuples_inserted,
        COALESCE(s.n_tup_upd, 0)::bigint AS tuples_updated,
        COALESCE(s.n_tup_del, 0)::bigint AS tuples_deleted,
        COALESCE(s.n_tup_hot_upd, 0)::bigint AS hot_updates,
        e.vacuum_threshold::double precision
            AS effective_vacuum_threshold,
        e.vacuum_scale_factor::double precision
            AS effective_vacuum_scale_factor,
        e.insert_threshold::double precision
            AS effective_insert_threshold,
        e.insert_scale_factor::double precision
            AS effective_insert_scale_factor,
        e.dead_tuple_trigger AS estimated_dead_tuple_trigger,
        e.insert_trigger AS estimated_insert_trigger,
        (e.dead_tuple_trigger IS NOT NULL
         AND COALESCE(s.n_dead_tup, 0) >= e.dead_tuple_trigger) AS dead_trigger_reached,
        (e.insert_trigger IS NOT NULL
         AND COALESCE(s.n_ins_since_vacuum, 0) >= e.insert_trigger) AS insert_trigger_reached,
        s.last_vacuum,
        s.last_autovacuum,
        d.stats_reset
    FROM thresholds AS e
    JOIN pg_namespace AS n
        ON n.oid = e.relnamespace
    LEFT JOIN pg_stat_all_tables AS s
        ON s.relid = e.oid
    CROSS JOIN db_stats AS d
    ORDER BY age(e.relfrozenxid) DESC
) AS q;
"#;

pub(super) const BACKENDS_SQL: &str = r#"
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        a.pid,
        a.datname::text AS database,
        a.usename::text AS username,
        NULLIF(a.application_name, '') AS application_name,
        a.state,
        age(a.backend_xmin)::bigint AS xmin_age,
        CASE
            WHEN a.xact_start IS NOT NULL
            THEN extract(
                epoch FROM clock_timestamp() - a.xact_start
            )::bigint
        END AS transaction_age_seconds,
        a.wait_event_type,
        a.wait_event
    FROM pg_stat_activity AS a
    WHERE a.backend_xmin IS NOT NULL
      AND a.pid <> pg_backend_pid() -- Do not report the collector's own snapshot as a blocker.
    ORDER BY age(a.backend_xmin) DESC
    LIMIT 10
) AS q;
"#;

pub(super) const SLOTS_SQL: &str = r#"
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        s.slot_name,
        s.slot_type,
        s.database::text AS database,
        s.active,
        CASE
            WHEN s.xmin IS NOT NULL
            THEN age(s.xmin)::bigint
        END AS xmin_age,
        CASE
            WHEN s.catalog_xmin IS NOT NULL
            THEN age(s.catalog_xmin)::bigint
        END AS catalog_xmin_age
    FROM pg_replication_slots AS s
    WHERE s.xmin IS NOT NULL
       OR s.catalog_xmin IS NOT NULL
    ORDER BY GREATEST(
        COALESCE(age(s.xmin), 0),
        COALESCE(age(s.catalog_xmin), 0)
    ) DESC
) AS q;
"#;

pub(super) const PREPARED_SQL: &str = r#"
SELECT json_build_object(
    'count', count(*),
    'oldest_age_seconds',
        CASE
            WHEN count(*) > 0
            THEN extract(
                epoch FROM clock_timestamp() - min(prepared)
            )::bigint
        END,
    'examples', (
        SELECT COALESCE(json_agg(row_to_json(p)), '[]'::json)
        FROM (
            SELECT gid::text AS gid,
                   transaction::text AS xid,
                   database::text AS database,
                   owner::text AS owner,
                   prepared::text AS prepared_at,
                   extract(epoch FROM clock_timestamp() - prepared)::bigint AS age_seconds
            FROM pg_prepared_xacts
            ORDER BY prepared
            LIMIT 10
        ) AS p
    )
)::text
FROM pg_prepared_xacts;
"#;

pub(super) const VACUUMS_SQL: &str = r#"
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        p.pid,
        c.oid::bigint AS relation_oid,
        p.datname::text AS database,
        n.nspname::text AS schema_name,
        c.relname::text AS relation_name,
        p.phase,
        age(c.relfrozenxid)::bigint AS xid_age,
        p.heap_blks_scanned,
        p.heap_blks_total,
        CASE
            WHEN p.heap_blks_total > 0
            THEN round(
                p.heap_blks_scanned::numeric
                / p.heap_blks_total::numeric
                * 100,
                1
            )::double precision
        END AS scan_pct,
        a.backend_type,
        COALESCE(
            a.backend_type = 'autovacuum worker'
            AND a.query ILIKE '%prevent wraparound%',
            false
        ) AS is_wraparound_vacuum,
        CASE WHEN a.query IS NULL OR a.query = '<insufficient privilege>'
                  OR a.backend_type IS NULL
            THEN 'unknown'
            ELSE 'query_text'
        END AS wraparound_detection,
        a.wait_event_type,
        a.wait_event
    FROM pg_stat_progress_vacuum AS p
    JOIN pg_class AS c
        ON c.oid = p.relid
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    LEFT JOIN pg_stat_activity AS a
        ON a.pid = p.pid
    WHERE p.datname = current_database()
    ORDER BY p.pid
) AS q;
"#;
