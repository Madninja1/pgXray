pub(super) const OVERVIEW_SQL: &str = r#"
SELECT json_build_object(
    'measured_at', clock_timestamp(),
    'database', current_database(),
    'database_oid', d.oid::bigint,
    'username', current_user,
    'server_version', current_setting('server_version'),
    'server_version_num', current_setting('server_version_num')::integer,
    'stats_reset', s.stats_reset,
    'autovacuum', current_setting('autovacuum') = 'on',
    'track_counts', current_setting('track_counts') = 'on',
    'autovacuum_max_workers', current_setting('autovacuum_max_workers')::integer,
    'autovacuum_naptime', current_setting('autovacuum_naptime'),
    'autovacuum_vacuum_threshold', current_setting('autovacuum_vacuum_threshold')::bigint,
    'autovacuum_vacuum_scale_factor', current_setting('autovacuum_vacuum_scale_factor')::double precision,
    'autovacuum_vacuum_max_threshold', nullif(current_setting('autovacuum_vacuum_max_threshold', true), '')::bigint,
    'autovacuum_vacuum_insert_threshold', current_setting('autovacuum_vacuum_insert_threshold')::bigint,
    'autovacuum_vacuum_insert_scale_factor', current_setting('autovacuum_vacuum_insert_scale_factor')::double precision,
    'autovacuum_analyze_threshold', current_setting('autovacuum_analyze_threshold')::bigint,
    'autovacuum_analyze_scale_factor', current_setting('autovacuum_analyze_scale_factor')::double precision,
    'autovacuum_vacuum_cost_limit', current_setting('autovacuum_vacuum_cost_limit')::bigint,
    'autovacuum_vacuum_cost_delay', current_setting('autovacuum_vacuum_cost_delay'),
    'vacuum_cost_limit', current_setting('vacuum_cost_limit')::bigint,
    'block_size', current_setting('block_size')::bigint,
    'log_autovacuum_min_duration', current_setting('log_autovacuum_min_duration')
)::text
FROM pg_database AS d
LEFT JOIN pg_stat_database AS s
    ON s.datid = d.oid
WHERE d.datname = current_database();
"#;

pub(super) const TABLES_SQL: &str = r#"
WITH global_cfg AS (
    SELECT
        current_setting('autovacuum') = 'on' AS global_autovacuum,
        current_setting('autovacuum_vacuum_threshold')::numeric AS vacuum_threshold,
        current_setting('autovacuum_vacuum_scale_factor')::numeric AS vacuum_scale_factor,
        -- Missing on PostgreSQL <=17. -1 disables the cap on PostgreSQL 18+.
        COALESCE(NULLIF(current_setting('autovacuum_vacuum_max_threshold', true), '')::numeric, -1) AS vacuum_max_threshold,
        current_setting('server_version_num')::integer AS server_version_num,
        current_setting('autovacuum_vacuum_insert_threshold')::numeric AS insert_threshold,
        current_setting('autovacuum_vacuum_insert_scale_factor')::numeric AS insert_scale_factor,
        current_setting('autovacuum_analyze_threshold')::numeric AS analyze_threshold,
        current_setting('autovacuum_analyze_scale_factor')::numeric AS analyze_scale_factor,
        CASE
            WHEN current_setting('autovacuum_vacuum_cost_limit')::integer = -1
            THEN current_setting('vacuum_cost_limit')::integer
            ELSE current_setting('autovacuum_vacuum_cost_limit')::integer
        END AS vacuum_cost_limit,
        (SELECT setting::numeric FROM pg_settings WHERE name = 'vacuum_cost_delay') AS regular_vacuum_cost_delay_ms,
        CASE
            WHEN (SELECT setting::numeric FROM pg_settings WHERE name = 'autovacuum_vacuum_cost_delay') < 0
            THEN (SELECT setting::numeric FROM pg_settings WHERE name = 'vacuum_cost_delay')
            ELSE (SELECT setting::numeric FROM pg_settings WHERE name = 'autovacuum_vacuum_cost_delay')
        END AS vacuum_cost_delay_ms,
        current_setting('autovacuum_freeze_max_age')::bigint AS freeze_max_age
),
base AS (
    SELECT
        c.oid,
        n.nspname,
        c.relname,
        c.relkind,
        c.reltuples,
        c.relpages,
        -- relallfrozen was added to pg_class in PostgreSQL 18.
        -- Reading the composite row as JSON keeps this SQL parseable on 16/17.
        COALESCE((to_jsonb(c)->>'relallfrozen')::integer, 0) AS relallfrozen,
        c.relfrozenxid,
        c.reltoastrelid,
        c.reloptions,
        s.n_live_tup,
        s.n_dead_tup,
        s.n_tup_ins,
        s.n_tup_upd,
        s.n_tup_del,
        s.n_tup_hot_upd,
        s.n_ins_since_vacuum,
        s.n_mod_since_analyze,
        s.last_vacuum,
        s.last_autovacuum,
        s.last_analyze,
        s.last_autoanalyze,
        s.vacuum_count,
        s.autovacuum_count,
        s.analyze_count,
        s.autoanalyze_count,
        parent.oid AS parent_oid,
        parent.nspname AS parent_schema,
        parent.relname AS parent_relation,
        pg_relation_size(c.oid)::bigint AS heap_size_bytes,
        pg_indexes_size(c.oid)::bigint AS indexes_size_bytes,
        pg_total_relation_size(c.oid)::bigint AS total_size_bytes,
        (
            SELECT count(*)::bigint
            FROM pg_index AS i
            WHERE i.indrelid = c.oid
        ) AS index_count
    FROM pg_stat_user_tables AS s
    JOIN pg_class AS c
        ON c.oid = s.relid
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    LEFT JOIN LATERAL (
        SELECT
            p.oid,
            pn.nspname,
            p.relname
        FROM pg_inherits AS inh
        JOIN pg_class AS p
            ON p.oid = inh.inhparent
        JOIN pg_namespace AS pn
            ON pn.oid = p.relnamespace
        WHERE inh.inhrelid = c.oid
        ORDER BY inh.inhseqno
        LIMIT 1
    ) AS parent
        ON TRUE
    WHERE c.relkind IN ('r', 'p')
    /* pgxray:table-oid-filter */
),
table_cfg AS (
    SELECT
        b.*,
        max(o.option_value) FILTER (WHERE o.option_name = 'autovacuum_enabled') AS table_autovacuum_enabled,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_threshold') AS table_vacuum_threshold,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_scale_factor') AS table_vacuum_scale_factor,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_max_threshold') AS table_vacuum_max_threshold,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_insert_threshold') AS table_insert_threshold,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_insert_scale_factor') AS table_insert_scale_factor,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_analyze_threshold') AS table_analyze_threshold,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_analyze_scale_factor') AS table_analyze_scale_factor,
        max(o.option_value::integer) FILTER (WHERE o.option_name = 'autovacuum_vacuum_cost_limit') AS table_vacuum_cost_limit,
        max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_cost_delay') AS table_vacuum_cost_delay_ms,
        max(o.option_value::bigint) FILTER (WHERE o.option_name = 'autovacuum_freeze_max_age') AS table_freeze_max_age
    FROM base AS b
    LEFT JOIN LATERAL pg_options_to_table(b.reloptions) AS o
        ON TRUE
    GROUP BY
        b.oid,
        b.nspname,
        b.relname,
        b.relkind,
        b.reltuples,
        b.relpages,
        b.relallfrozen,
        b.relfrozenxid,
        b.reltoastrelid,
        b.reloptions,
        b.n_live_tup,
        b.n_dead_tup,
        b.n_tup_ins,
        b.n_tup_upd,
        b.n_tup_del,
        b.n_tup_hot_upd,
        b.n_ins_since_vacuum,
        b.n_mod_since_analyze,
        b.last_vacuum,
        b.last_autovacuum,
        b.last_analyze,
        b.last_autoanalyze,
        b.vacuum_count,
        b.autovacuum_count,
        b.analyze_count,
        b.autoanalyze_count,
        b.parent_oid,
        b.parent_schema,
        b.parent_relation,
        b.heap_size_bytes,
        b.indexes_size_bytes,
        b.total_size_bytes,
        b.index_count
),
effective AS (
    SELECT
        t.*,
        g.*,
        g.global_autovacuum
            AND COALESCE(t.table_autovacuum_enabled::boolean, true)
            AS regular_autovacuum_enabled,
        COALESCE(t.table_vacuum_threshold, g.vacuum_threshold) AS effective_vacuum_threshold,
        COALESCE(t.table_vacuum_scale_factor, g.vacuum_scale_factor) AS effective_vacuum_scale_factor,
        COALESCE(t.table_vacuum_max_threshold, g.vacuum_max_threshold) AS effective_vacuum_max_threshold,
        COALESCE(t.table_insert_threshold, g.insert_threshold) AS effective_insert_threshold,
        COALESCE(t.table_insert_scale_factor, g.insert_scale_factor) AS effective_insert_scale_factor,
        COALESCE(t.table_analyze_threshold, g.analyze_threshold) AS effective_analyze_threshold,
        COALESCE(t.table_analyze_scale_factor, g.analyze_scale_factor) AS effective_analyze_scale_factor,
        CASE
            WHEN t.table_vacuum_cost_limit IS NULL
                THEN g.vacuum_cost_limit
            WHEN t.table_vacuum_cost_limit = -1
                THEN current_setting('vacuum_cost_limit')::integer
            ELSE t.table_vacuum_cost_limit
        END AS effective_vacuum_cost_limit,
        CASE
            WHEN t.table_vacuum_cost_delay_ms IS NULL
                THEN g.vacuum_cost_delay_ms
            WHEN t.table_vacuum_cost_delay_ms < 0
                THEN g.regular_vacuum_cost_delay_ms
            ELSE t.table_vacuum_cost_delay_ms
        END AS effective_vacuum_cost_delay_ms,
        COALESCE(t.table_freeze_max_age, g.freeze_max_age) AS effective_freeze_max_age
    FROM table_cfg AS t
    CROSS JOIN global_cfg AS g
),
triggers AS (
    SELECT
        e.*,
        -- The PostgreSQL worker uses > (strict), not >=. These fields
        -- represent the first integer counter value that can trigger work.
        floor(
            CASE WHEN e.effective_vacuum_max_threshold >= 0
                THEN LEAST(e.effective_vacuum_max_threshold,
                    e.effective_vacuum_threshold
                    + e.effective_vacuum_scale_factor * GREATEST(e.reltuples::numeric, 0))
                ELSE e.effective_vacuum_threshold
                    + e.effective_vacuum_scale_factor * GREATEST(e.reltuples::numeric, 0)
            END
        )::bigint + 1 AS dead_tuple_trigger,
        CASE
            WHEN e.effective_insert_threshold < 0 THEN NULL
            ELSE floor(
                e.effective_insert_threshold
                + e.effective_insert_scale_factor * GREATEST(e.reltuples::numeric, 0)
                * CASE
                    WHEN e.server_version_num >= 180000
                         AND e.relpages > 0 AND e.relallfrozen > 0
                    THEN 1 - LEAST(e.relallfrozen, e.relpages)::numeric / e.relpages
                    ELSE 1
                  END
            )::bigint + 1
        END AS insert_trigger,
        floor(
            e.effective_analyze_threshold
            + e.effective_analyze_scale_factor * GREATEST(e.reltuples::numeric, 0)
        )::bigint + 1 AS analyze_trigger
    FROM effective AS e
)
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        t.oid::bigint AS relation_oid,
        t.nspname::text AS schema_name,
        t.relname::text AS relation_name,
        CASE t.relkind
            WHEN 'p' THEN 'partitioned_table'
            ELSE 'table'
        END AS relation_type,
        t.parent_oid::bigint AS parent_oid,
        t.parent_schema::text AS parent_schema,
        t.parent_relation::text AS parent_relation,
        t.heap_size_bytes,
        t.indexes_size_bytes,
        t.total_size_bytes,
        t.index_count,
        GREATEST(round(t.reltuples)::bigint, 0) AS estimated_rows,
        COALESCE(t.n_live_tup, 0)::bigint AS live_tuples,
        COALESCE(t.n_dead_tup, 0)::bigint AS dead_tuples,
        round(
            100.0::numeric
            * COALESCE(t.n_dead_tup, 0)::numeric
            / NULLIF(
                COALESCE(t.n_live_tup, 0)::numeric
                + COALESCE(t.n_dead_tup, 0)::numeric,
                0
            ),
            2
        )::double precision AS dead_tuple_pct,
        COALESCE(t.n_tup_ins, 0)::bigint AS tuples_inserted,
        COALESCE(t.n_tup_upd, 0)::bigint AS tuples_updated,
        COALESCE(t.n_tup_del, 0)::bigint AS tuples_deleted,
        COALESCE(t.n_tup_hot_upd, 0)::bigint AS hot_updates,
        round(
            100.0::numeric
            * COALESCE(t.n_tup_hot_upd, 0)::numeric
            / NULLIF(COALESCE(t.n_tup_upd, 0)::numeric, 0),
            1
        )::double precision AS hot_update_pct,
        COALESCE(t.n_ins_since_vacuum, 0)::bigint AS inserts_since_vacuum,
        COALESCE(t.n_mod_since_analyze, 0)::bigint AS modifications_since_analyze,
        t.last_vacuum,
        t.last_autovacuum,
        t.last_analyze,
        t.last_autoanalyze,
        COALESCE(t.vacuum_count, 0)::bigint AS vacuum_count,
        COALESCE(t.autovacuum_count, 0)::bigint AS autovacuum_count,
        COALESCE(t.analyze_count, 0)::bigint AS analyze_count,
        COALESCE(t.autoanalyze_count, 0)::bigint AS autoanalyze_count,
        CASE
            WHEN t.relkind = 'p' THEN NULL
            ELSE age(t.relfrozenxid)::bigint
        END AS xid_age,
        t.reloptions,
        toast.data AS toast,
        t.regular_autovacuum_enabled AS autovacuum_enabled,
        t.relkind <> 'p' AS autovacuum_applicable,
        t.effective_vacuum_threshold::double precision,
        t.effective_vacuum_scale_factor::double precision,
        NULLIF(t.effective_vacuum_max_threshold, -1)::bigint AS effective_vacuum_max_threshold,
        t.server_version_num,
        GREATEST(t.reltuples::double precision, 0) AS reltuples_estimate,
        CASE WHEN t.server_version_num >= 180000 THEN
            CASE WHEN t.relpages > 0 AND t.relallfrozen > 0
                THEN 1 - LEAST(t.relallfrozen, t.relpages)::double precision / t.relpages
                ELSE 1.0 END
        ELSE NULL END AS insert_unfrozen_fraction,
        t.effective_insert_threshold::double precision,
        t.effective_insert_scale_factor::double precision,
        t.effective_analyze_threshold::double precision,
        t.effective_analyze_scale_factor::double precision,
        t.effective_vacuum_cost_limit::bigint,
        t.effective_vacuum_cost_delay_ms::double precision,
        t.effective_freeze_max_age::bigint,
        t.dead_tuple_trigger,
        t.insert_trigger,
        t.analyze_trigger,
        round(
            COALESCE(t.n_dead_tup, 0)::numeric
            / NULLIF(t.dead_tuple_trigger::numeric, 0)
            * 100,
            1
        )::double precision AS dead_trigger_used_pct,
        round(
            COALESCE(t.n_ins_since_vacuum, 0)::numeric
            / NULLIF(t.insert_trigger::numeric, 0)
            * 100,
            1
        )::double precision AS insert_trigger_used_pct,
        round(
            COALESCE(t.n_mod_since_analyze, 0)::numeric
            / NULLIF(t.analyze_trigger::numeric, 0)
            * 100,
            1
        )::double precision AS analyze_trigger_used_pct,
        t.relkind <> 'p'
            AND COALESCE(t.n_dead_tup, 0) >= t.dead_tuple_trigger AS dead_trigger_reached,
        t.relkind <> 'p'
            AND t.insert_trigger IS NOT NULL
            AND COALESCE(t.n_ins_since_vacuum, 0) >= t.insert_trigger AS insert_trigger_reached,
        t.relkind <> 'p'
            AND COALESCE(t.n_mod_since_analyze, 0) >= t.analyze_trigger AS analyze_trigger_reached,
        CASE
            WHEN COALESCE(t.n_tup_ins, 0) = 0
             AND COALESCE(t.n_tup_upd, 0) = 0
             AND COALESCE(t.n_tup_del, 0) = 0
                THEN 'static_or_no_activity'
            WHEN COALESCE(t.n_tup_ins, 0) > 0
             AND COALESCE(t.n_tup_upd, 0) = 0
             AND COALESCE(t.n_tup_del, 0) = 0
                THEN 'append_only_observed'
            WHEN COALESCE(t.n_tup_upd, 0) + COALESCE(t.n_tup_del, 0)
                 >= COALESCE(t.n_tup_ins, 0)
                THEN 'update_delete_heavy'
            ELSE 'mixed'
        END AS workload_profile
    FROM triggers AS t
    LEFT JOIN LATERAL (
        WITH toast_cfg AS (
            SELECT
                max(o.option_value) FILTER (WHERE o.option_name = 'autovacuum_enabled') AS autovacuum_enabled,
                max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_threshold') AS vacuum_threshold,
                max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_scale_factor') AS vacuum_scale_factor,
                max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_max_threshold') AS vacuum_max_threshold,
                max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_insert_threshold') AS insert_threshold,
                max(o.option_value::numeric) FILTER (WHERE o.option_name = 'autovacuum_vacuum_insert_scale_factor') AS insert_scale_factor,
                max(o.option_value::bigint) FILTER (WHERE o.option_name = 'autovacuum_freeze_max_age') AS freeze_max_age
            FROM pg_class AS tc0
            LEFT JOIN LATERAL pg_options_to_table(tc0.reloptions) AS o
                ON TRUE
            WHERE tc0.oid = t.reltoastrelid
        ),
        toast_base AS (
            SELECT
                tc.oid,
                tn.nspname,
                tc.relname,
                tc.reltuples,
                tc.relpages,
                -- PG16/17 have no pg_class.relallfrozen column.
                COALESCE((to_jsonb(tc)->>'relallfrozen')::integer, 0) AS relallfrozen,
                tc.relfrozenxid,
                tc.reloptions,
                COALESCE(ts.n_live_tup, 0)::bigint AS n_live_tup,
                COALESCE(ts.n_dead_tup, 0)::bigint AS n_dead_tup,
                COALESCE(ts.n_ins_since_vacuum, 0)::bigint AS n_ins_since_vacuum,
                ts.last_autovacuum,
                COALESCE(ts.autovacuum_count, 0)::bigint AS autovacuum_count,
                t.global_autovacuum
                    AND COALESCE(cfg.autovacuum_enabled::boolean, true)
                    AS autovacuum_enabled,
                COALESCE(cfg.vacuum_threshold, t.vacuum_threshold) AS vacuum_threshold,
                COALESCE(cfg.vacuum_scale_factor, t.vacuum_scale_factor) AS vacuum_scale_factor,
                COALESCE(cfg.vacuum_max_threshold, t.vacuum_max_threshold) AS vacuum_max_threshold,
                t.server_version_num,
                COALESCE(cfg.insert_threshold, t.insert_threshold) AS insert_threshold,
                COALESCE(cfg.insert_scale_factor, t.insert_scale_factor) AS insert_scale_factor,
                COALESCE(cfg.freeze_max_age, t.freeze_max_age) AS freeze_max_age
            FROM pg_class AS tc
            JOIN pg_namespace AS tn
                ON tn.oid = tc.relnamespace
            LEFT JOIN pg_stat_all_tables AS ts
                ON ts.relid = tc.oid
            CROSS JOIN toast_cfg AS cfg
            WHERE tc.oid = t.reltoastrelid
              AND t.reltoastrelid <> 0
        ),
        toast_triggers AS (
            SELECT
                b.*,
                floor(
                    CASE WHEN b.vacuum_max_threshold >= 0
                        THEN LEAST(b.vacuum_max_threshold,
                            b.vacuum_threshold
                            + b.vacuum_scale_factor * GREATEST(b.reltuples::numeric, 0))
                        ELSE b.vacuum_threshold
                            + b.vacuum_scale_factor * GREATEST(b.reltuples::numeric, 0)
                    END
                )::bigint + 1 AS dead_trigger,
                CASE
                    WHEN b.insert_threshold < 0 THEN NULL
                    ELSE floor(
                        b.insert_threshold
                        + b.insert_scale_factor * GREATEST(b.reltuples::numeric, 0)
                        * CASE
                            WHEN b.server_version_num >= 180000
                                 AND b.relpages > 0 AND b.relallfrozen > 0
                            THEN 1 - LEAST(b.relallfrozen, b.relpages)::numeric / b.relpages
                            ELSE 1
                          END
                    )::bigint + 1
                END AS insert_trigger
            FROM toast_base AS b
        )
        SELECT json_build_object(
            'relation_oid', x.oid::bigint,
            'schema_name', x.nspname::text,
            'relation_name', x.relname::text,
            'total_size_bytes', pg_total_relation_size(x.oid)::bigint,
            'estimated_rows', GREATEST(round(x.reltuples)::bigint, 0),
            'live_tuples', x.n_live_tup,
            'dead_tuples', x.n_dead_tup,
            'inserts_since_vacuum', x.n_ins_since_vacuum,
            'last_autovacuum', x.last_autovacuum,
            'autovacuum_count', x.autovacuum_count,
            'xid_age', age(x.relfrozenxid)::bigint,
            'reloptions', x.reloptions,
            'autovacuum_enabled', x.autovacuum_enabled,
            'effective_vacuum_threshold', x.vacuum_threshold::double precision,
            'effective_vacuum_scale_factor', x.vacuum_scale_factor::double precision,
            'effective_vacuum_max_threshold', NULLIF(x.vacuum_max_threshold, -1)::bigint,
            'server_version_num', x.server_version_num,
            'reltuples_estimate', GREATEST(x.reltuples::double precision, 0),
            'insert_unfrozen_fraction',
                CASE WHEN x.server_version_num >= 180000 THEN
                    CASE WHEN x.relpages > 0 AND x.relallfrozen > 0
                        THEN 1 - LEAST(x.relallfrozen, x.relpages)::double precision / x.relpages
                        ELSE 1.0 END
                ELSE NULL END,
            'effective_insert_threshold', x.insert_threshold::double precision,
            'effective_insert_scale_factor', x.insert_scale_factor::double precision,
            'effective_freeze_max_age', x.freeze_max_age::bigint,
            'dead_tuple_trigger', x.dead_trigger,
            'insert_trigger', x.insert_trigger,
            'dead_trigger_used_pct', round(
                x.n_dead_tup::numeric / NULLIF(x.dead_trigger::numeric, 0) * 100,
                1
            )::double precision,
            'insert_trigger_used_pct', round(
                x.n_ins_since_vacuum::numeric / NULLIF(x.insert_trigger::numeric, 0) * 100,
                1
            )::double precision,
            'dead_trigger_reached', x.n_dead_tup >= x.dead_trigger,
            'insert_trigger_reached', x.insert_trigger IS NOT NULL AND x.n_ins_since_vacuum >= x.insert_trigger
        ) AS data
        FROM toast_triggers AS x
    ) AS toast
        ON TRUE
    ORDER BY
        GREATEST(
            COALESCE(
                COALESCE(t.n_dead_tup, 0)::numeric
                / NULLIF(t.dead_tuple_trigger::numeric, 0),
                0
            ),
            COALESCE(
                COALESCE(t.n_ins_since_vacuum, 0)::numeric
                / NULLIF(t.insert_trigger::numeric, 0),
                0
            ),
            COALESCE(
                COALESCE(t.n_mod_since_analyze, 0)::numeric
                / NULLIF(t.analyze_trigger::numeric, 0),
                0
            )
        ) DESC,
        t.total_size_bytes DESC,
        t.nspname,
        t.relname
) AS q;
"#;

// Keep the complete collection for the overview, but avoid calculating
// sizes for every relation when the user opens one table or applies a policy.
// relation_oid is a u32, not user-provided SQL text.
// An explicit SQL insertion point lives in the base CTE, before relation
// sizes are evaluated. Fail loudly if the query is refactored and the marker
// disappears: silently running the unfiltered query would be expensive.
pub(super) fn table_sql(relation_oid: u32) -> Result<String, String> {
    const FILTER_MARKER: &str = "/* pgxray:table-oid-filter */";
    let (before, after) = TABLES_SQL
        .split_once(FILTER_MARKER)
        .ok_or(crate::i18n::text(
            "Не удалось подготовить SQL для одной таблицы: отсутствует OID-фильтр",
            "Failed to prepare SQL for a single table: missing OID filter",
        ))?;
    if after.contains(FILTER_MARKER) {
        return Err(crate::i18n::text(
            "Не удалось подготовить SQL для одной таблицы: OID-фильтр неоднозначен",
            "Failed to prepare SQL for a single table: ambiguous OID filter",
        )
        .into());
    }
    Ok(format!("{before}AND c.oid = {relation_oid}::oid\n{after}"))
}

pub(super) const VACUUM_PROGRESS_SQL: &str = r#"
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        p.pid,
        p.relid::bigint AS relation_oid,
        n.nspname::text AS schema_name,
        c.relname::text AS relation_name,
        p.phase::text,
        p.heap_blks_total::bigint,
        p.heap_blks_scanned::bigint,
        p.heap_blks_vacuumed::bigint,
        round(
            p.heap_blks_scanned::numeric
            / NULLIF(p.heap_blks_total::numeric, 0)
            * 100,
            1
        )::double precision AS scan_pct,
        round(
            p.heap_blks_vacuumed::numeric
            / NULLIF(p.heap_blks_total::numeric, 0)
            * 100,
            1
        )::double precision AS vacuum_pct,
        p.index_vacuum_count::bigint,
        -- pg_stat_progress_vacuum changed in PostgreSQL 17. Use JSON keys
        -- to avoid referring to columns absent from the server's catalog.
        (to_jsonb(p)->>'max_dead_tuples')::bigint AS max_dead_tuples,
        (to_jsonb(p)->>'num_dead_tuples')::bigint AS num_dead_tuples,
        (to_jsonb(p)->>'max_dead_tuple_bytes')::bigint AS max_dead_tuple_bytes,
        (to_jsonb(p)->>'dead_tuple_bytes')::bigint AS dead_tuple_bytes,
        (to_jsonb(p)->>'num_dead_item_ids')::bigint AS num_dead_item_ids,
        a.backend_type::text,
        COALESCE(a.backend_type = 'autovacuum worker', false) AS is_autovacuum,
        CASE
            WHEN a.query_start IS NULL THEN NULL
            ELSE GREATEST(
                0,
                extract(epoch FROM clock_timestamp() - a.query_start)::bigint
            )
        END AS query_age_seconds,
        a.wait_event_type::text,
        a.wait_event::text
    FROM pg_stat_progress_vacuum AS p
    JOIN pg_class AS c
        ON c.oid = p.relid
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    LEFT JOIN pg_stat_activity AS a
        ON a.pid = p.pid
    WHERE p.datid = (SELECT oid FROM pg_database WHERE datname = current_database())
    ORDER BY a.query_start NULLS LAST, p.pid
) AS q;
"#;

pub(super) const ANALYZE_PROGRESS_SQL: &str = r#"
SELECT COALESCE(
    json_agg(row_to_json(q)),
    '[]'::json
)::text
FROM (
    SELECT
        p.pid,
        p.relid::bigint AS relation_oid,
        n.nspname::text AS schema_name,
        c.relname::text AS relation_name,
        p.phase::text,
        p.sample_blks_total::bigint,
        p.sample_blks_scanned::bigint,
        round(
            p.sample_blks_scanned::numeric
            / NULLIF(p.sample_blks_total::numeric, 0)
            * 100,
            1
        )::double precision AS sample_pct,
        p.ext_stats_total::bigint,
        p.ext_stats_computed::bigint,
        p.child_tables_total::bigint,
        p.child_tables_done::bigint,
        a.backend_type::text,
        COALESCE(a.backend_type = 'autovacuum worker', false) AS is_autoanalyze,
        CASE
            WHEN a.query_start IS NULL THEN NULL
            ELSE GREATEST(
                0,
                extract(epoch FROM clock_timestamp() - a.query_start)::bigint
            )
        END AS query_age_seconds,
        a.wait_event_type::text,
        a.wait_event::text
    FROM pg_stat_progress_analyze AS p
    JOIN pg_class AS c
        ON c.oid = p.relid
    JOIN pg_namespace AS n
        ON n.oid = c.relnamespace
    LEFT JOIN pg_stat_activity AS a
        ON a.pid = p.pid
    WHERE p.datid = (SELECT oid FROM pg_database WHERE datname = current_database())
    ORDER BY a.query_start NULLS LAST, p.pid
) AS q;
"#;

pub(super) fn indexes_sql(relation_oid: u32) -> String {
    format!(
        r#"
        SELECT COALESCE(
            json_agg(row_to_json(q)),
            '[]'::json
        )::text
        FROM (
            SELECT
                idx.oid::bigint AS index_oid,
                idx.relname::text AS index_name,
                i.indisprimary AS is_primary,
                i.indisunique AS is_unique,
                i.indexprs IS NOT NULL AS has_expression,
                i.indpred IS NOT NULL AS is_partial,
                pg_relation_size(idx.oid)::bigint AS index_size_bytes,
                COALESCE(
                    (
                        SELECT json_agg(
                            COALESCE(a.attname, '<expression>')
                            ORDER BY k.ordinality
                        )
                        FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ordinality)
                        LEFT JOIN pg_attribute AS a
                            ON a.attrelid = i.indrelid
                           AND a.attnum = k.attnum
                        WHERE k.ordinality <= i.indnkeyatts
                    ),
                    '[]'::json
                ) AS indexed_columns,
                COALESCE(
                    (
                        SELECT json_agg(
                            COALESCE(a.attname, '<expression>')
                            ORDER BY k.ordinality
                        )
                        FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ordinality)
                        LEFT JOIN pg_attribute AS a
                            ON a.attrelid = i.indrelid
                           AND a.attnum = k.attnum
                        WHERE k.ordinality > i.indnkeyatts
                    ),
                    '[]'::json
                ) AS included_columns,
                pg_get_expr(i.indpred, i.indrelid)::text AS predicate
            FROM pg_index AS i
            JOIN pg_class AS idx
                ON idx.oid = i.indexrelid
            WHERE i.indrelid = {relation_oid}::oid
            ORDER BY
                i.indisprimary DESC,
                i.indisunique DESC,
                idx.relname
        ) AS q;
        "#,
    )
}

pub(super) fn update_statements_sql(relation_oid: u32) -> String {
    format!(
        r#"
    WITH target AS (
        SELECT relname FROM pg_class WHERE oid = {relation_oid}::oid
    )
    SELECT COALESCE(json_agg(row_to_json(q)), '[]'::json)::text
    FROM (
        SELECT
            s.queryid::text AS queryid,
            s.userid::bigint AS userid,
            s.dbid::bigint AS dbid,
            s.toplevel AS toplevel,
            s.calls::bigint,
            s.total_exec_time::double precision AS total_exec_time_ms,
            s.mean_exec_time::double precision AS mean_exec_time_ms,
            s.rows::bigint,
            s.query AS sql_text
        FROM pg_stat_statements AS s
        CROSS JOIN target AS t
        WHERE s.dbid = (SELECT oid FROM pg_database WHERE datname = current_database())
          AND s.query ~* '(^|[^[:alnum:]_])UPDATE[[:space:]]'
          AND length(s.query) <= 8192
          AND position(lower(t.relname) in lower(s.query)) > 0
        ORDER BY s.calls DESC, s.total_exec_time DESC
        LIMIT 200
    ) AS q;
    "#
    )
}
