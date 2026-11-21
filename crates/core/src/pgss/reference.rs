use super::{model::*, queries::*};

pub fn reference() -> PgssReference {
    PgssReference {
        methodology: methodology(),
        queries: query_reference(),
        sources: source_reference(),
    }
}

fn methodology() -> Vec<PgssMethodStep> {
    vec![
        method_step(
            "availability",
            crate::i18n::text("Проверить доступность pg_stat_statements", "Check pg_stat_statements availability"),
            crate::i18n::text("Сначала фиксируем extension, preload, compute_query_id и версию сервера.", "First record the extension, preload configuration, compute_query_id, and server version."),
            &[
                crate::i18n::text("Extension создаётся отдельно в каждой базе, а library должна быть загружена через shared_preload_libraries.", "The extension is created separately in each database, and the library must be loaded through shared_preload_libraries."),
                crate::i18n::text("Не пытаться диагностировать dealloc через пустые/недоступные views.", "Do not diagnose dealloc using empty or unavailable views."),
            ],
        ),
        method_step(
            "health",
            crate::i18n::text("Снять health baseline", "Capture a health baseline"),
            crate::i18n::text("Фиксируем stats_reset, dealloc, max/current entries и заполненность.", "Record stats_reset, dealloc, maximum/current entries, and utilization."),
            &[
                crate::i18n::text("Высокий fill_pct сам по себе не является проблемой.", "A high fill_pct is not necessarily a problem."),
                crate::i18n::text("Абсолютный dealloc без временного интервала не показывает churn rate.", "The absolute dealloc counter does not show churn rate without a time interval."),
            ],
        ),
        method_step(
            "cardinality-owner",
            crate::i18n::text("Локализовать владельца cardinality", "Identify the source of cardinality"),
            crate::i18n::text("Группируем entries по role/database без вывода SQL.", "Group entries by role and database without exposing SQL."),
            &[
                crate::i18n::text("Смотрим entries, calls=1, calls<=5 и total_calls.", "Examine entries, calls=1, calls<=5, and total_calls."),
                crate::i18n::text("Большая доля one-shot entries делает группу кандидатом для дальнейшего анализа.", "A large share of one-shot entries makes the group a candidate for further analysis."),
            ],
        ),
        method_step(
            "command",
            crate::i18n::text("Разбить cardinality по command", "Break down cardinality by command"),
            crate::i18n::text("Группируем по началу SQL; WITH/CTE и недоступные query text — отдельные неизвестные классы, без угадывания DML.", "Group by SQL prefix; WITH/CTE and unavailable query text are separate unknown classes, without guessing DML."),
            &[
                crate::i18n::text("Command extraction использует query text внутри PostgreSQL, но наружу возвращает только command. <HIDDEN> означает отсутствие доступа к тексту.", "Command extraction uses query text inside PostgreSQL but returns only the command; <HIDDEN> means the text cannot be accessed."),
                crate::i18n::text("CALL при track_utility=on требует особого внимания из-за textual matching utility statements.", "CALL requires special attention with track_utility=on because utility statements are matched textually."),
            ],
        ),
        method_step(
            "call-target",
            crate::i18n::text("Локализовать high-cardinality CALL", "Find high-cardinality CALL statements"),
            crate::i18n::text("Для видимых CALL извлекаем текст перед первой открывающей скобкой и возвращаем hash-tag; это не OID процедуры.", "For visible CALL statements, extract the text before the first opening parenthesis and return a hash tag; it is not a procedure OID."),
            &[
                crate::i18n::text("Один target, занимающий почти весь max и состоящий почти полностью из calls=1, — сильный признак utility churn.", "A single target using almost all available entries, nearly all with calls=1, strongly suggests utility statement churn."),
                crate::i18n::text("Driver/client-side binding подтверждается уже вне pg_stat_statements.", "Verify driver or client-side binding outside pg_stat_statements."),
            ],
        ),
        method_step(
            "history",
            crate::i18n::text("Сравнить snapshots", "Compare snapshots"),
            crate::i18n::text("Главный health-сигнал — dealloc delta/rate при неизменном stats_reset.", "The key health signal is dealloc delta/rate with an unchanged stats_reset."),
            &[
                crate::i18n::text("Если stats_reset изменился, cumulative dealloc нельзя вычитать.", "If stats_reset changed, cumulative dealloc values cannot be subtracted."),
                crate::i18n::text("Fill/one-shot — snapshot gauges и могут сравниваться как состояние, но не как monotonic counters.", "Fill/one-shot are snapshot gauges and can be compared as state, but not as monotonic counters."),
            ],
        ),
        method_step(
            "performance",
            crate::i18n::text("Посмотреть performance hotspots", "Inspect performance hotspots"),
            crate::i18n::text("Отдельно смотрим total/mean execution, temp, reads, WAL, variance и planning.", "Examine total and mean execution time, temporary I/O, reads, WAL, variance, and planning separately."),
            &[
                crate::i18n::text("TOP total и TOP mean отвечают на разные вопросы.", "TOP by total time and TOP by mean time answer different questions."),
                crate::i18n::text("Temp/read/WAL — направления следующего расследования, а не автоматическая причина.", "Temp I/O, reads, and WAL guide further investigation but do not prove a cause."),
            ],
        ),
        method_step(
            "statement-history",
            crate::i18n::text("Проверить динамику конкретного statement", "Inspect the history of an individual statement"),
            crate::i18n::text("Для sampled queryid сравниваем cumulative counters между соседними сохранёнными snapshots.", "For a sampled queryid, compare cumulative counters between consecutive saved snapshots."),
            &[
                crate::i18n::text("Confirmed delta требует тот же stats_reset, неизменный dealloc и monotonic counters.", "A confirmed delta requires the same stats_reset, unchanged dealloc, and monotonic counters."),
                crate::i18n::text("Если statement выпал из TOP sample, pgXray показывает gap и не интерпретирует его как отсутствие PGSS entry.", "If a statement falls out of the TOP sample, pgXray shows a gap rather than claiming that the PGSS entry is absent."),
                crate::i18n::text("Interval mean execution считается из total_exec_time delta / calls delta, а не из cumulative mean gauge.", "Interval mean execution time is calculated as the total_exec_time delta divided by the calls delta, not from a cumulative mean gauge."),
            ],
        ),
        method_step(
            "statement-change",
            crate::i18n::text("Сравнить два подтверждённых statement intervals", "Compare two confirmed statement intervals"),
            crate::i18n::text("Change Analysis сравнивает нормализованные interval metrics, а не cumulative gauges.", "Change Analysis compares normalized interval metrics, not cumulative gauges."),
            &[
                crate::i18n::text("Для current анализа нужны три последовательных sampled points и два safe intervals подряд.", "The current analysis requires three consecutive sampled points and two consecutive safe intervals."),
                crate::i18n::text("Сравниваются calls/hour, interval execution per call, workload share, reads/temp/WAL per call и planning per plan.", "Compare calls per hour, interval execution per call, workload share, reads/temp/WAL per call, and planning per plan."),
                crate::i18n::text("Confirmed change означает арифметически подтверждённое изменение; причина остаётся не установленной до Plan/Locks/I/O/Table evidence.", "A confirmed change is numerically supported; its cause remains unknown until plan, lock, I/O, or table evidence is examined."),
            ],
        ),
        method_step(
            "drilldown",
            crate::i18n::text("Открыть конкретный queryid только при необходимости", "Open a specific queryid only when needed"),
            crate::i18n::text("Broad snapshot не выгружает полный SQL; выбранный entry можно запросить отдельно.", "The broad snapshot does not export full SQL; a selected entry can be retrieved separately."),
            &[
                crate::i18n::text("Query entry может исчезнуть между snapshot и drill-down из-за deallocation/reset.", "A query entry may disappear between snapshot and drill-down due to deallocation or reset."),
                crate::i18n::text("После drill-down дальнейший шаг может быть EXPLAIN, lock/table analysis или проверка приложения.", "After drill-down, investigate with EXPLAIN, lock/table analysis, or application checks."),
            ],
        ),
    ]
}

fn query_reference() -> Vec<PgssQueryReference> {
    vec![
        query(
            "overview",
            crate::i18n::text("PGSS availability и config", "PGSS availability and configuration"),
            crate::i18n::text("Проверяет extension, preload и основные настройки без обращения к pg_stat_statements view.", "Checks extension availability, preload, and main settings without querying the pg_stat_statements view."),
            &[
                "server/database identity",
                "extension version",
                "shared_preload_libraries",
                "compute_query_id",
                "max / track / track_utility / track_planning / save",
                "track_io_timing",
            ],
            &[
                crate::i18n::text("Extension может быть установлен только в текущей базе.", "The extension may be installed only in the current database."),
                "Preload — server-level requirement.",
            ],
            OVERVIEW_SQL,
        ),
        query(
            "stats",
            "pg_stat_statements_info",
            crate::i18n::text("Фиксирует dealloc и начало текущего PGSS statistics period.", "Records dealloc and the start of the current PGSS statistics period."),
            &[
                "dealloc",
                "stats_reset",
                "stats_age_seconds",
            ],
            &[
                crate::i18n::text("dealloc — число deallocation events, а не число удалённых SQL entries.", "dealloc counts deallocation events, not removed SQL entries."),
                crate::i18n::text("Для churn нужен delta/rate между snapshots.", "Assess churn using deltas or rates between snapshots."),
            ],
            STATS_SQL,
        ),
        query(
            "capacity",
            crate::i18n::text("Capacity и one-shot distribution", "Capacity and one-shot distribution"),
            crate::i18n::text("Показывает заполненность configured entry capacity и долю редко выполняемых entries.", "Shows configured entry capacity utilization and the share of rarely executed entries."),
            &[
                "max_entries",
                "current_entries / fill_pct",
                "one_call_entries / pct",
                "calls<=5 entries / pct",
                "total_calls",
                "total_exec_time_ms для workload-share interval math",
            ],
            &[
                crate::i18n::text("Fill 95–100% без растущего dealloc не является самостоятельной проблемой.", "Utilization of 95–100% without increasing dealloc is not a problem by itself."),
                crate::i18n::text("One-shot distribution помогает понять retention pressure.", "One-shot distribution helps assess retention pressure."),
            ],
            CAPACITY_SQL,
        ),
        query(
            "role-database",
            crate::i18n::text("Cardinality по role/database", "Cardinality by role and database"),
            crate::i18n::text("Локализует владельца большого количества entries без query text.", "Identifies which role and database produce many entries without query text."),
            &[
                "userid / dbid",
                "role / database",
                "entries / share",
                "one-call / calls<=5",
                "total_calls",
            ],
            &[
                crate::i18n::text("Начинать расследование удобно с крупнейшей группы.", "Start investigating with the largest group."),
                crate::i18n::text("Это распределение текущего sample, а не historical attribution dealloc events.", "This is the distribution of the current sample, not historical attribution of deallocation events."),
            ],
            ROLE_DATABASE_SQL,
        ),
        query(
            "commands",
            crate::i18n::text("Cardinality по command", "Cardinality by command"),
            crate::i18n::text("Показывает SQL/utility command distribution без возврата полного query text.", "Shows SQL and utility command distribution without returning full query text."),
            &[
                "role/database",
                "command",
                "entries / share",
                "one-call / calls<=5",
                "total_calls",
            ],
            &[
                crate::i18n::text("<HIDDEN> означает недоступный текст SQL; <OTHER> — нераспознанное начало; <WITH/CTE> — тип DML не определён.", "<HIDDEN> means SQL text is unavailable; <OTHER> is an unrecognized prefix; <WITH/CTE> means the DML type is undetermined."),
                crate::i18n::text("Доминирующий CALL при track_utility=on — повод проверить CALL target aggregation.", "Dominant CALL usage with track_utility=on warrants checking CALL target aggregation."),
            ],
            COMMANDS_SQL,
        ),
        query(
            "call-targets",
            "CALL target aggregation",
            crate::i18n::text("Удаляет аргументы CALL внутри PostgreSQL, хеширует target и считает cardinality.", "Strips CALL arguments inside PostgreSQL, hashes the target, and counts cardinality."),
            &[
                "call_tag",
                "role/database",
                "entries",
                crate::i18n::text("% от pg_stat_statements.max", "% of pg_stat_statements.max"),
                "one-call %",
                "total_calls",
            ],
            &[
                crate::i18n::text("Hash-tag группирует извлечённую текстовую метку, не доказывает идентичность процедуры и не является анонимизацией.", "The hash tag groups an extracted text label; it does not prove procedure identity or provide anonymization."),
                crate::i18n::text("Очень высокая доля capacity + почти все calls=1 — сильный признак high-cardinality CALL.", "High capacity utilization with almost all entries at calls=1 strongly suggests high-cardinality CALL."),
            ],
            CALL_TARGETS_SQL,
        ),
        query(
            "performance",
            "Performance samples",
            crate::i18n::text("Снимает TOP-25 UI rankings и отдельный bounded history sample без query text через pg_stat_statements(false).", "Collects TOP-25 UI rankings and a separate bounded history sample without query text using pg_stat_statements(false)."),
            &[
                "total/mean/min/max/stddev execution",
                "planning statistics",
                "rows",
                "shared buffers",
                "temp blocks",
                "I/O timing",
                "WAL",
                "queryid identity if visible",
                "history_sample: до 300 queryid из top-50 по нескольким performance dimensions",
            ],
            &[
                crate::i18n::text("UI rankings остаются TOP-25; history_sample используется для более устойчивой statement history.", "UI rankings remain TOP-25; history_sample supports more robust statement history."),
                crate::i18n::text("Даже расширенный history sample не представляет полный pg_stat_statements и может пропускать редкие statements.", "Even the expanded history sample does not represent all pg_stat_statements entries and may omit rare statements."),
                crate::i18n::text("Нули planning при выключенном track_planning не являются измерением затрат планирования.", "Zero planning values with track_planning disabled are not measurements of planning costs."),
            ],
            PERFORMANCE_SQL,
        ),
    ]
}

fn source_reference() -> Vec<PgssSourceReference> {
    vec![
        source(
            "pgss",
            "pg_stat_statements",
            crate::i18n::text("Официальное описание view, pg_stat_statements_info, normalization, dealloc и configuration parameters.", "Official documentation of the view, pg_stat_statements_info, normalization, dealloc, and configuration parameters."),
            "pgstatstatements.html",
        ),
        source(
            "monitoring",
            "Monitoring Database Activity",
            crate::i18n::text("Контекст cumulative statistics и reset semantics PostgreSQL.", "Background on PostgreSQL cumulative statistics and reset semantics."),
            "monitoring-stats.html",
        ),
        source(
            "preload",
            "Client Connection Defaults / Shared Library Preloading",
            crate::i18n::text("Описание shared_preload_libraries.", "Documentation for shared_preload_libraries."),
            "runtime-config-client.html#RUNTIME-CONFIG-CLIENT-PRELOAD",
        ),
    ]
}

fn method_step(id: &str, title: &str, purpose: &str, how_to_read: &[&str]) -> PgssMethodStep {
    PgssMethodStep {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        how_to_read: owned_strings(how_to_read),
    }
}

fn query(
    id: &str,
    title: &str,
    purpose: &str,
    output: &[&str],
    how_to_read: &[&str],
    sql: &str,
) -> PgssQueryReference {
    PgssQueryReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        output: owned_strings(output),
        how_to_read: owned_strings(how_to_read),
        sql: sql.trim().to_owned(),
    }
}

fn source(id: &str, title: &str, purpose: &str, path: &str) -> PgssSourceReference {
    PgssSourceReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        path: path.to_owned(),
    }
}

fn owned_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
