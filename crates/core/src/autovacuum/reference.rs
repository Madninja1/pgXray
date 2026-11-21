use super::{model::*, queries::*};

pub fn reference() -> AutovacuumReference {
    AutovacuumReference {
        methodology: methodology(),
        queries: query_reference(),
        sources: source_reference(),
    }
}

fn methodology() -> Vec<AutovacuumMethodStep> {
    vec![
        method_step(
            "profile",
            crate::i18n::text("Оценить состояние таблицы и характер изменений", "Assess table condition and change patterns"),
            crate::i18n::text("Сначала изучить размер таблицы, INSERT/UPDATE/DELETE, оценку мёртвых строк, HOT, индексы и даты обслуживания.", "First examine table size, INSERT/UPDATE/DELETE, estimated dead tuples, HOT, indexes, and maintenance timestamps."),
            &[
                crate::i18n::text("n_dead_tup и reltuples — оценки PostgreSQL, а не точный размер раздувания (bloat).", "n_dead_tup and reltuples are PostgreSQL estimates, not an exact measurement of bloat."),
                crate::i18n::text("Профиль нагрузки построен по накопительным счётчикам и не заменяет наблюдение за выбранный интервал.", "The workload profile uses cumulative counters and does not replace measurements over a selected interval."),
                crate::i18n::text("Отдельно проверить основную таблицу, её партиции и TOAST, когда они присутствуют.", "Check the parent table, partitions, and TOAST relations separately when present."),
            ],
        ),
        method_step(
            "effective-settings",
            crate::i18n::text("Определить действующие настройки и пороги", "Determine effective settings and thresholds"),
            crate::i18n::text("pgXray объединяет глобальные параметры сервера с настройками отдельной таблицы (reloptions). Для VACUUM, VACUUM по вставкам и ANALYZE вычисляются разные пороги.", "pgXray combines global server settings with per-table reloptions. It calculates separate thresholds for VACUUM, insert-triggered VACUUM, and ANALYZE."),
            &[
                crate::i18n::text("Для обычного VACUUM: порог = vacuum_threshold + vacuum_scale_factor × reltuples. В PostgreSQL 18+ результат может ограничиваться autovacuum_vacuum_max_threshold (-1 означает без ограничения).", "For ordinary VACUUM: threshold = vacuum_threshold + vacuum_scale_factor × reltuples. PostgreSQL 18+ may cap it at autovacuum_vacuum_max_threshold (-1 means no cap)."),
                crate::i18n::text("Для VACUUM по вставкам: порог = insert_threshold + insert_scale_factor × reltuples; в PostgreSQL 18+ масштабируемая часть дополнительно учитывает долю незамороженных страниц. При insert_threshold = -1 этот механизм отключён.", "For insert-triggered VACUUM: threshold = insert_threshold + insert_scale_factor × reltuples. In PostgreSQL 18+, the scaled component also accounts for the fraction of unfrozen pages. An insert_threshold of -1 disables this trigger."),
                crate::i18n::text("Для ANALYZE: порог = analyze_threshold + analyze_scale_factor × reltuples. Autoanalyze рассматривается независимо от VACUUM.", "For ANALYZE: threshold = analyze_threshold + analyze_scale_factor × reltuples. Autoanalyze is assessed independently of VACUUM."),
                crate::i18n::text("В pgXray trigger — первое целое значение счётчика, строго превышающее вычисленный порог: floor(порог) + 1; достижение trigger не означает немедленный старт worker.", "In pgXray, the trigger is the first integer counter value strictly above the calculated threshold: floor(threshold) + 1. Reaching the trigger does not guarantee an immediate worker start."),
            ],
        ),
        method_step(
            "history",
            crate::i18n::text("Сравнить несколько наблюдений", "Compare multiple observations"),
            crate::i18n::text("Скорость изменений и регулярность автоматического обслуживания оцениваются по истории снимков, а не по одному накопленному значению.", "Estimate change rates and automatic maintenance regularity from snapshot history, not a single cumulative value."),
            &[
                crate::i18n::text("Накопительные счётчики сравниваются только при известном совпадающем stats_reset. Если он неизвестен, достоверную скорость по истории не рассчитываем.", "Compare cumulative counters only when stats_reset is known and identical. Without that evidence, do not calculate a reliable historical rate."),
                crate::i18n::text("Если между снимками был VACUUM, изменение оценки dead tuples не равно скорости накопления мёртвых строк.", "If VACUUM ran between snapshots, the change in estimated dead tuples is not the rate at which dead tuples accumulated."),
                crate::i18n::text("Ручные VACUUM и ANALYZE не доказывают работу автоматического обслуживания.", "Manual VACUUM and ANALYZE do not demonstrate that automatic maintenance is working."),
            ],
        ),
        method_step(
            "advisor",
            crate::i18n::text("Отделить задержку обслуживания от настройки порогов", "Separate maintenance delays from threshold configuration"),
            crate::i18n::text("Advisor отдельно исследует обычный VACUUM, VACUUM по вставкам, ANALYZE и обслуживание TOAST.", "Advisor investigates ordinary VACUUM, insert-triggered VACUUM, ANALYZE, and TOAST maintenance separately."),
            &[
                crate::i18n::text("Если расчётный порог давно достигнут, а автоматический цикл не наблюдается, сначала проверить занятость workers, блокировки и ограничения обслуживания.", "If the calculated threshold was reached long ago but no automatic cycle was observed, first examine worker occupancy, locks, and maintenance limits."),
                crate::i18n::text("Кандидаты на изменение настроек появляются только при достаточной непрерывной истории; сценарии 75% и 50% требуют проверки DBA.", "Configuration candidates require sufficiently continuous history; the 75% and 50% scenarios require DBA review."),
                crate::i18n::text("Показатели доступности разделов тоже важны: ошибка или таймаут сбора не означают отсутствие проблем.", "Section availability matters: collection errors or timeouts do not mean there are no problems."),
            ],
        ),
        method_step(
            "hot",
            crate::i18n::text("Проверить HOT-обновления и индексы", "Inspect HOT updates and indexes"),
            crate::i18n::text("Низкая доля HOT может сопровождаться дополнительной работой с индексами при частых UPDATE.", "A low HOT ratio may entail additional index maintenance during frequent UPDATEs."),
            &[
                crate::i18n::text("Проверить, какие столбцы изменяются и входят ли они в индексы.", "Check which columns are updated and whether they are indexed."),
                crate::i18n::text("Не изменять fillfactor автоматически только потому, что HOT равен нулю.", "Do not change fillfactor automatically merely because the HOT ratio is zero."),
            ],
        ),
        method_step(
            "policy",
            crate::i18n::text("Проверить расчёт и только затем рассматривать применение", "Verify the calculation before considering application"),
            crate::i18n::text("Ручная симуляция позволяет задать желаемое абсолютное число мёртвых строк; Advisor предлагает отдельные сценарии на основе истории.", "Manual simulation lets you specify an absolute dead-tuple target; Advisor proposes separate history-based scenarios."),
            &[
                crate::i18n::text("При reltuples > 0 приближённый коэффициент = (желаемый порог - 0,5 - vacuum_threshold) / reltuples. Отступ 0,5 помогает попасть между целыми значениями; итоговый trigger пересчитывается как floor(порог) + 1.", "For reltuples > 0, approximate scale factor = (desired threshold - 0.5 - vacuum_threshold) / reltuples. The 0.5 offset targets a value between integers; the trigger is recalculated as floor(threshold) + 1."),
                crate::i18n::text("Это расчётная симуляция, а не гарантия частоты VACUUM: в PostgreSQL 18+ ограничение autovacuum_vacuum_max_threshold может сделать изменение scale_factor неэффективным.", "This is a numerical simulation, not a guarantee of VACUUM frequency. In PostgreSQL 18+, the autovacuum_vacuum_max_threshold cap may make changing scale_factor ineffective."),
                crate::i18n::text("При недостаточной оценке строк или желаемом пороге ниже фиксированного threshold коэффициент нельзя надёжно подобрать одним scale_factor.", "If the row estimate is insufficient or the target is below the fixed threshold, scale_factor alone cannot reliably achieve it."),
                crate::i18n::text("Ничего не применять автоматически. При явном guarded Apply backend повторно проверяет объект, текущие настройки и выполняющееся обслуживание перед ALTER TABLE.", "Do not apply anything automatically. On explicit guarded Apply, the backend rechecks the relation, current settings, and ongoing maintenance before ALTER TABLE."),
            ],
        ),
    ]
}

const REFERENCE_RELATION_OID: u32 = u32::MAX;

// These are SQL templates for the reference view. They must not suggest
// that an example OID identifies a real table on the user's server.
fn relation_sql_template(sql: String) -> String {
    sql.replace(
        &format!("{REFERENCE_RELATION_OID}::oid"),
        "<relation_oid>::oid",
    )
}

fn query_reference() -> Vec<AutovacuumQueryReference> {
    let table_detail_sql = table_sql(REFERENCE_RELATION_OID)
        .expect("TABLES_SQL must contain exactly one relation filter anchor");
    let table_detail_sql = relation_sql_template(table_detail_sql);
    let index_detail_sql = relation_sql_template(indexes_sql(REFERENCE_RELATION_OID));
    let update_evidence_sql = relation_sql_template(update_statements_sql(REFERENCE_RELATION_OID));

    vec![
        query(
            "overview",
            crate::i18n::text("Настройки сервера и контекст статистики", "Server settings and statistics context"),
            crate::i18n::text("Показывает выбранную базу, версию PostgreSQL, время сброса статистики и глобальные настройки Autovacuum.", "Shows the selected database, PostgreSQL version, statistics reset time, and global Autovacuum settings."),
            &[
                "autovacuum / track_counts",
                crate::i18n::text("версия PostgreSQL и stats_reset", "PostgreSQL version and stats_reset"),
                crate::i18n::text("настройки VACUUM, INSERT-VACUUM, ANALYZE и ограничений ресурсов", "VACUUM, insert-triggered VACUUM, ANALYZE, and resource-limit settings"),
            ],
            &[
                crate::i18n::text("stats_reset задаёт известную границу для накопительных счётчиков текущей базы.", "stats_reset provides a known boundary for cumulative counters in the current database."),
                crate::i18n::text("Индивидуальные reloptions таблицы могут переопределять глобальные настройки.", "Per-table reloptions may override global settings."),
            ],
            OVERVIEW_SQL,
        ),
        query(
            "tables",
            crate::i18n::text("Состояние всех пользовательских таблиц", "All user table conditions"),
            crate::i18n::text("Собирает статистику таблиц, их размеры, обслуживание, TOAST и действующие расчётные пороги.", "Collects table statistics, sizes, maintenance, TOAST, and effective calculated thresholds."),
            &[
                crate::i18n::text("размеры и приблизительное число строк", "sizes and approximate row counts"),
                crate::i18n::text("INSERT, UPDATE, DELETE, HOT и статистика VACUUM/ANALYZE", "INSERT, UPDATE, DELETE, HOT, and VACUUM/ANALYZE statistics"),
                crate::i18n::text("параметры reloptions и пороги VACUUM, INSERT-VACUUM и ANALYZE", "reloptions and thresholds for VACUUM, insert-triggered VACUUM, and ANALYZE"),
            ],
            &[
                crate::i18n::text("n_dead_tup — оценка, а не точный объём bloat.", "n_dead_tup is an estimate, not an exact measurement of bloat."),
                crate::i18n::text("trigger — первое целое значение счётчика, строго превышающее расчётный порог; это не доказательство задержки worker.", "The trigger is the first integer counter value strictly above the calculated threshold; it does not prove a worker is delayed."),
                crate::i18n::text("PostgreSQL 18+ учитывает ограничение autovacuum_vacuum_max_threshold и долю незамороженных страниц для INSERT-VACUUM, включая TOAST.", "PostgreSQL 18+ accounts for the autovacuum_vacuum_max_threshold cap and the unfrozen-page fraction for insert-triggered VACUUM, including TOAST."),
                crate::i18n::text("Полный обзор включает все пользовательские таблицы и рассчитывает размеры. При таймауте или превышении лимита результат неполон, а не равен нулю.", "The full overview includes all user tables and computes sizes. On timeout or limit exceedance, results are incomplete, not zero."),
            ],
            TABLES_SQL,
        ),
        query(
            "table-detail",
            crate::i18n::text("Состояние одной таблицы", "Single table condition"),
            crate::i18n::text("Использует тот же расчёт, что полный обзор, но ограничивает выборку конкретным OID до вычисления размеров.", "Uses the same calculations as the full overview but filters by a specific OID before computing sizes."),
            &[crate::i18n::text("сведения выбранной таблицы, её настройки, триггеры и TOAST", "selected table details, settings, triggers, and TOAST")],
            &[
                crate::i18n::text("<relation_oid> — обозначение OID выбранной таблицы, не готовое SQL-значение.", "<relation_oid> denotes the selected table OID; it is not a ready-to-use SQL value."),
                crate::i18n::text("В приложении фактический OID подставляется безопасным числовым значением.", "The application substitutes the actual OID as a validated numeric value."),
            ],
            &table_detail_sql,
        ),
        query(
            "vacuum-progress",
            crate::i18n::text("Выполняющиеся VACUUM", "Running VACUUM operations"),
            crate::i18n::text("Показывает текущие ручные и автоматические VACUUM в выбранной базе.", "Shows current manual and automatic VACUUM operations in the selected database."),
            &[crate::i18n::text("фаза выполнения", "execution phase"), crate::i18n::text("сканирование и очистка heap", "heap scanning and cleanup"), crate::i18n::text("счётчики мёртвых строк или байтов", "dead-tuple or byte counters"), crate::i18n::text("ожидание worker", "worker waiting")],
            &[
                crate::i18n::text("Проценты сканирования и очистки относятся к разным фазам, а не ко всей операции.", "Scanning and cleanup percentages refer to separate phases, not the whole operation."),
                crate::i18n::text("В PostgreSQL 16 доступны счётчики dead tuples, а в PostgreSQL 17+ — счётчики байтов и идентификаторов; единицы различаются.", "PostgreSQL 16 provides dead-tuple counters, while PostgreSQL 17+ provides byte and identifier counters; units differ."),
            ],
            VACUUM_PROGRESS_SQL,
        ),
        query(
            "analyze-progress",
            crate::i18n::text("Выполняющиеся ANALYZE", "Running ANALYZE operations"),
            crate::i18n::text("Показывает текущие ручные и автоматические ANALYZE.", "Shows current manual and automatic ANALYZE operations."),
            &[crate::i18n::text("фаза выполнения", "execution phase"), crate::i18n::text("блоки выборки", "sample blocks"), crate::i18n::text("расширенная статистика", "extended statistics"), crate::i18n::text("дочерние таблицы", "child tables")],
            &[
                crate::i18n::text("Autoanalyze работает по порогу изменений строк, отдельно от порога VACUUM.", "Autoanalyze uses a row-change threshold separate from VACUUM."),
                crate::i18n::text("Текущий прогресс не показывает частоту завершённых ANALYZE; для неё нужны счётчики и история.", "Current progress does not indicate how often ANALYZE completes; counters and history are needed."),
            ],
            ANALYZE_PROGRESS_SQL,
        ),
        query(
            "table-indexes",
            crate::i18n::text("Индексы выбранной таблицы", "Indexes of the selected table"),
            crate::i18n::text("Возвращает состав и размеры индексов указанной таблицы, отдельно ключевые и INCLUDE-столбцы.", "Returns index definitions and sizes for the selected table, distinguishing key and INCLUDE columns."),
            &[crate::i18n::text("OID и имя индекса", "index OID and name"), "PRIMARY/UNIQUE", crate::i18n::text("ключевые и INCLUDE-столбцы", "key and INCLUDE columns"), crate::i18n::text("размер и предикат", "size and predicate")],
            &[crate::i18n::text("<relation_oid> нужно заменить на OID реальной таблицы; это шаблон SQL.", "<relation_oid> must be replaced with the real table OID; this is an SQL template.")],
            &index_detail_sql,
        ),
        query(
            "update-evidence",
            crate::i18n::text("Примеры UPDATE-нагрузки из pg_stat_statements", "Sample UPDATE workload from pg_stat_statements"),
            crate::i18n::text("Находит потенциально связанные UPDATE по квалифицированному имени таблицы и возвращает агрегированные показатели без текста SQL.", "Finds potentially related UPDATE statements by qualified table name and returns aggregated metrics without SQL text."),
            &["queryid", crate::i18n::text("количество вызовов", "call count"), crate::i18n::text("общее и среднее время выполнения", "total and mean execution time"), crate::i18n::text("обработанные строки", "processed rows")],
            &[
                crate::i18n::text("<relation_oid> нужно заменить на OID таблицы; это шаблон SQL.", "<relation_oid> must be replaced with the table OID; this is an SQL template."),
                crate::i18n::text("Сопоставление имени по тексту не является полноценным разбором SQL: часть запросов может быть пропущена.", "Matching names in text is not a full SQL parse; some queries may be missed."),
                crate::i18n::text("Полный текст SQL скрыт в результате, но внутри запроса pg_stat_statements используется для фильтрации; не публикуйте диагностические SQL без проверки конфиденциальности.", "Full SQL text is hidden in the result, but pg_stat_statements text is used internally for filtering; do not publish diagnostic SQL without a privacy review."),
            ],
            &update_evidence_sql,
        ),
    ]
}

fn source_reference() -> Vec<AutovacuumSourceReference> {
    vec![
        source(
            "routine-vacuuming",
            "Обычное обслуживание VACUUM",
            "Механизм VACUUM/ANALYZE, формулы порогов и защита от wraparound.",
            "routine-vacuuming.html",
        ),
        source(
            "autovacuum-settings",
            "Настройки Autovacuum",
            "Глобальные параметры и пороги автоматического обслуживания.",
            "runtime-config-autovacuum.html",
        ),
        source(
            "storage-parameters",
            "Параметры таблиц",
            "Индивидуальные настройки Autovacuum через reloptions.",
            "sql-createtable.html#SQL-CREATETABLE-STORAGE-PARAMETERS",
        ),
        source(
            "statistics",
            "Статистика PostgreSQL",
            "Накопительные счётчики pg_stat_user_tables и pg_stat_database.",
            "monitoring-stats.html",
        ),
        source(
            "progress",
            "Прогресс операций",
            "Представления pg_stat_progress_vacuum и pg_stat_progress_analyze.",
            "progress-reporting.html",
        ),
        source(
            "pg-class",
            "Каталог pg_class",
            "Оценки reltuples, параметры reloptions и свойства отношений.",
            "catalog-pg-class.html",
        ),
    ]
}

fn method_step(id: &str, title: &str, purpose: &str, how_to_read: &[&str]) -> AutovacuumMethodStep {
    AutovacuumMethodStep {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        how_to_read: how_to_read
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    }
}

fn query(
    id: &str,
    title: &str,
    purpose: &str,
    output: &[&str],
    how_to_read: &[&str],
    sql: &str,
) -> AutovacuumQueryReference {
    AutovacuumQueryReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        output: output.iter().map(|value| (*value).to_owned()).collect(),
        how_to_read: how_to_read
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        sql: sql.to_owned(),
    }
}

fn source(id: &str, title: &str, purpose: &str, path: &str) -> AutovacuumSourceReference {
    AutovacuumSourceReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        path: path.to_owned(),
    }
}
