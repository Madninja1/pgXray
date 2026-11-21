use super::{
    collector::{XID_COLLECTION_BUDGET, XID_MAX_SECTION_ROWS, XID_PAGE_SIZE},
    model::*,
    queries::*,
};

pub fn reference() -> XidReference {
    XidReference {
        methodology: methodology(),
        queries: query_reference(),
        sources: source_reference(),
    }
}

fn methodology() -> Vec<XidMethodStep> {
    vec![
        method_step(
            "freeze-settings",
            crate::i18n::text("Проверить действующие настройки заморозки", "Check the effective freeze settings"),
            crate::i18n::text("Сначала фиксируем версию PostgreSQL и настройки, относительно которых оценивается возраст идентификаторов транзакций.", "First record the PostgreSQL version and settings used to assess transaction ID age."),
            &[
                crate::i18n::text("`autovacuum_freeze_max_age` — порог защитного VACUUM от переполнения XID, а не физический предел пространства XID.", "`autovacuum_freeze_max_age` is the anti-wraparound VACUUM threshold, not the physical limit of the XID space."),
                crate::i18n::text("`vacuum_freeze_table_age` не запускает autovacuum: он влияет на агрессивность уже запущенного VACUUM.", "`vacuum_freeze_table_age` does not start autovacuum: it affects the aggressiveness of a VACUUM that is already running."),
                crate::i18n::text("`age(relfrozenxid)` — возраст горизонта заморозки отношения в XID, а не число строк или операций над таблицей.", "`age(relfrozenxid)` is the relation freeze horizon age in XIDs, not the number of rows or table operations."),
            ],
        ),
        method_step(
            "oldest-relations",
            crate::i18n::text("Найти отношения с наибольшим возрастом XID", "Find relations with the oldest XIDs"),
            crate::i18n::text("Собираем обычные таблицы, материализованные представления и TOAST; итоговый список сортируется по возрасту XID. Сбор ограничен по времени и объёму.", "Collects ordinary tables, materialized views, and TOAST relations, sorted by XID age. Collection is bounded by time and size."),
            &[
                crate::i18n::text("Смотрим возраст XID, размер, последние VACUUM/autovacuum и `reloptions`.", "Examine XID age, size, recent VACUUM/autovacuum, and reloptions."),
                crate::i18n::text("Для отношения с индивидуальной настройкой процент считается относительно действующего порога этой таблицы.", "For relations with per-table settings, the percentage uses that table's effective threshold."),
                crate::i18n::text("Большой возраст XID сам по себе не доказывает неисправность autovacuum.", "A high XID age alone does not prove that autovacuum has failed."),
            ],
        ),
        method_step(
            "toast-parent",
            crate::i18n::text("Проверить TOAST и основную таблицу", "Check TOAST and its parent table"),
            crate::i18n::text("Если самым старым оказалось TOAST-отношение, устанавливаем его связь с основной таблицей.", "If a TOAST relation is the oldest, identify its parent table."),
            &[
                crate::i18n::text("Сравниваем `xid_age` TOAST и основной таблицы.", "Compare the xid_age of TOAST and its parent table."),
                crate::i18n::text("Не приписываем возраст TOAST основной таблице: горизонты заморозки у них различаются.", "Do not attribute the TOAST age to its parent: their freeze horizons differ."),
            ],
        ),
        method_step(
            "database-age",
            crate::i18n::text("Сравнить возраст баз данных", "Compare database ages"),
            crate::i18n::text("Смотрим `age(datfrozenxid)` и `mxid_age(datminmxid)` для баз с `datallowconn=true`; базы без разрешения подключений не входят в выборку.", "Examine age(datfrozenxid) and mxid_age(datminmxid) for databases with datallowconn=true; databases that disallow connections are excluded."),
            &[
                crate::i18n::text("Если возраст базы близок к возрасту самого старого отношения, оно может определять горизонт заморозки базы.", "If the database age is close to the oldest relation's age, that relation may determine the database freeze horizon."),
                crate::i18n::text("Проценты XID и MultiXact рассчитываются относительно разных глобальных порогов и не означают долю до физического переполнения счётчика.", "XID and MultiXact percentages use different global thresholds; neither represents the fraction remaining before physical counter wraparound."),
            ],
        ),
        method_step(
            "multixact-age",
            crate::i18n::text("Отдельно проверить возраст MultiXact", "Check MultiXact age separately"),
            crate::i18n::text("XID и MultiXact — два самостоятельных 32-битных механизма PostgreSQL: MultiXact хранит сведения о нескольких блокировках одной строки и стареет независимо от XID.", "XID and MultiXact are separate 32-bit PostgreSQL mechanisms. MultiXact tracks multiple row lockers and ages independently of XID."),
            &[
                crate::i18n::text("`age(relfrozenxid)` измеряет возраст XID; `mxid_age(relminmxid)` — возраст MultiXact для конкретного отношения.", "age(relfrozenxid) measures XID age; mxid_age(relminmxid) measures MultiXact age for a relation."),
                crate::i18n::text("`mxid_age(datminmxid)` показывает возраст MultiXact на уровне базы. Сравниваем его с `autovacuum_multixact_freeze_max_age`, а не с XID-порогом.", "mxid_age(datminmxid) is database-level MultiXact age; compare it with autovacuum_multixact_freeze_max_age, not the XID threshold."),
                crate::i18n::text("VACUUM замораживает старые XID и обслуживает старые MultiXact; принудительное обслуживание возможно по любому из двух механизмов.", "VACUUM freezes old XIDs and handles old MultiXacts; either mechanism may trigger forced maintenance."),
                crate::i18n::text("`all_frozen_pages` доступно только в PostgreSQL 18; отсутствие поля в 16/17 не означает ноль полностью замороженных страниц.", "all_frozen_pages is available only in PostgreSQL 18; its absence in 16/17 does not mean zero fully frozen pages."),
            ],
        ),
        method_step(
            "blockers",
            crate::i18n::text("Проверить возможное удержание горизонта очистки", "Investigate possible cleanup horizon retention"),
            crate::i18n::text("Смотрим `backend_xmin`, слоты репликации и подготовленные транзакции.", "Examine backend_xmin, replication slots, and prepared transactions."),
            &[
                crate::i18n::text("Большой возраст `backend_xmin` вместе с длинной транзакцией может указывать на удержание старого снимка.", "Old backend_xmin together with a long transaction may indicate an old snapshot is being held."),
                crate::i18n::text("Старые `xmin`/`catalog_xmin` слота — повод для проверки, но не доказательство причины.", "Old slot xmin or catalog_xmin warrants investigation but does not prove the cause."),
                crate::i18n::text("Нельзя завершать процесс или удалять слот только по одному показателю XID age.", "Do not terminate a process or drop a slot based on XID age alone."),
            ],
        ),
        method_step(
            "vacuum",
            crate::i18n::text("Проверить текущий VACUUM", "Check currently running VACUUM"),
            crate::i18n::text("Проверяем, идёт ли VACUUM для старых отношений и удалось ли распознать его как защитный от wraparound.", "Check whether VACUUM is running on old relations and whether it can be identified as anti-wraparound maintenance."),
            &[
                crate::i18n::text("`scan_pct` — прогресс сканирования heap, а не завершённость всего VACUUM.", "scan_pct indicates heap scan progress, not completion of the entire VACUUM."),
                crate::i18n::text("Если отношение уже обслуживается VACUUM, не запускаем второй VACUUM без отдельного расследования.", "If VACUUM already runs on a relation, do not launch another one without further investigation."),
                crate::i18n::text("Отсутствие строк с достигнутыми обычными порогами не исключает защитного VACUUM по возрасту XID/MultiXact.", "No rows crossing ordinary thresholds does not rule out anti-wraparound VACUUM triggered by XID or MultiXact age."),
            ],
        ),
        method_step(
            "normal-triggers",
            crate::i18n::text("Проверить условия запуска обычного autovacuum", "Check ordinary autovacuum triggers"),
            crate::i18n::text("Отдельно проверяем условия запуска по мёртвым версиям и вставкам для старейших отношений.", "Check dead-tuple and insert triggers separately for the oldest relations."),
            &[
                crate::i18n::text("Расчёт учитывает глобальные настройки и переопределения `reloptions` для таблиц.", "The calculation accounts for global settings and per-table reloptions overrides."),
                crate::i18n::text("Недостижение обоих обычных порогов при большом XID age возможно для редко изменяемой таблицы и не отключает принудительный VACUUM.", "A rarely changed table can remain below both ordinary thresholds despite high XID age; this does not disable forced VACUUM."),
                crate::i18n::text("Достигнутый порог без VACUUM — повод исследовать доступность worker-процессов, блокировки, другие базы, журналы и I/O.", "A reached threshold without VACUUM warrants examining worker availability, locks, other databases, logs, and I/O."),
            ],
        ),
        method_step(
            "xid-rate",
            crate::i18n::text("Накопить историю для оценки скорости изменения XID", "Collect history to estimate the XID consumption rate"),
            crate::i18n::text("Каждый снимок фиксирует время сервера и верхнюю границу `pg_snapshot_xmax(pg_current_snapshot())` без назначения XID read-only транзакции (для совместимости JSON поле называется `current_xid`).", "Each snapshot records server time and the upper bound pg_snapshot_xmax(pg_current_snapshot()) without assigning an XID to the read-only transaction (the JSON field is named current_xid for compatibility)."),
            &[
                crate::i18n::text("Скорость можно оценить только по нескольким снимкам за известный интервал; граница снимка не является XID самой диагностической транзакции.", "Estimate a rate only from multiple snapshots over a known interval; the snapshot boundary is not the diagnostic transaction's own XID."),
                crate::i18n::text("Короткий интервал не подходит для долгосрочного прогноза ресурса XID.", "A short interval is not suitable for long-term XID exhaustion forecasts."),
                crate::i18n::text("Для устойчивой оценки нужен ряд измерений за несколько часов или сутки; непрерывность накопительных VACUUM-счётчиков требует известного `stats_reset`.", "A robust estimate requires observations over several hours or a day; continuity of cumulative VACUUM counters requires known stats_reset."),
            ],
        ),
        method_step(
            "policy-advisor",
            crate::i18n::text("Проверить историю перед изменением настроек таблицы", "Check history before changing table settings"),
            crate::i18n::text("Advisor различает обслуживание с продвижением горизонта заморозки, VACUUM без продвижения горизонта и редко изменяемые таблицы без циклов VACUUM.", "Advisor distinguishes maintenance that advances the freeze horizon, VACUUM without horizon progress, and rarely changed tables with no VACUUM cycles."),
            &[
                crate::i18n::text("Кандидат с числовыми настройками не строится по одному снимку: нужны сопоставимые наблюдения.", "A numerical policy candidate cannot be based on a single snapshot; comparable observations are required."),
                crate::i18n::text("`autovacuum_freeze_table_age` влияет на агрессивный режим уже запущенного VACUUM; `autovacuum_freeze_max_age` задаёт порог принудительного обслуживания по XID.", "autovacuum_freeze_table_age influences aggressive mode in an already running VACUUM; autovacuum_freeze_max_age defines the forced XID maintenance threshold."),
                crate::i18n::text("ANALYZE и доля all-visible страниц — контекст, а не доказательство продвижения `relfrozenxid` или `relminmxid`.", "ANALYZE and the fraction of all-visible pages provide context, not proof that relfrozenxid or relminmxid advanced."),
            ],
        ),
        method_step(
            "collection-limits",
            crate::i18n::text("Проверить полноту диагностического снимка", "Check diagnostic snapshot completeness"),
            &format!(
                "Сбор XID состоит из последовательных read-only запросов с общим бюджетом {} секунд. Отношения и условия autovacuum читаются страницами по {} OID, не более {} записей на секцию.",
                XID_COLLECTION_BUDGET.as_secs(), XID_PAGE_SIZE, XID_MAX_SECTION_ROWS
            ),
            &[
                crate::i18n::text("Разные секции могут отражать разные моменты времени; время завершения каждой секции фиксируется отдельно на стороне pgXray.", "Different sections may represent different moments; pgXray records each section's completion time separately."),
                crate::i18n::text("Если превышен бюджет, размер ответа или предел строк, секция возвращает ошибку. Неполная выборка не считается успешным полным снимком.", "A section returns an error when time, response size, or row limits are exceeded. An incomplete result is not treated as a complete snapshot."),
                crate::i18n::text("SQL в справочнике для отношений и условий autovacuum — исходный шаблон; Collector добавляет ограничение по OID, сортировку и LIMIT к каждой странице.", "Reference SQL for relations and autovacuum triggers is a template; Collector adds OID bounds, ordering, and LIMIT to each page."),
            ],
        ),
    ]
}

fn query_reference() -> Vec<XidQueryReference> {
    vec![
        query(
            "overview",
            "Версия PostgreSQL, настройки заморозки и позиция XID",
            "Фиксирует контекст снимка, XID/MultiXact-возраст текущей базы и наблюдаемую границу XID для истории.",
            &[
                "measured_at",
                "database / database_oid",
                "username",
                "server_version / server_version_num",
                "current_xid",
                "autovacuum",
                "autovacuum_freeze_max_age",
                "vacuum_freeze_table_age",
                "vacuum_freeze_min_age",
                "autovacuum_max_workers",
                "autovacuum_vacuum_cost_limit / delay",
                "database_xid_age",
                "database_mxid_age",
                "autovacuum_multixact_freeze_max_age",
                "stats_reset",
            ],
            &[
                "Поле `current_xid` хранит `pg_snapshot_xmax(pg_current_snapshot())`, верхнюю границу снимка, а не XID собственной транзакции. Само по себе оно не задаёт скорость.",
                "`stats_reset = NULL` не подтверждает непрерывность накопительных счётчиков VACUUM; такие интервалы не считаются доказательством для Policy.",
            ],
            OVERVIEW_SQL,
        ),
        query(
            "databases",
            "Возраст XID и MultiXact баз данных",
            "Показывает возраст `datfrozenxid` и `datminmxid` для подключаемых баз кластера.",
            &[
                "database_oid",
                "database",
                "xid_age",
                "pct_of_global_freeze_max",
                "mxid_age",
                "pct_of_global_multixact_freeze_max",
            ],
            &[
                "Сопоставляем возраст базы с возрастом старейших отношений; список включает только базы с `datallowconn=true`.",
                "Проценты XID и MultiXact относятся к разным глобальным freeze-порогам, а не к физическому пределу переполнения.",
            ],
            DATABASES_SQL,
        ),
        query(
            "relations",
            "Возраст XID и MultiXact отношений",
            &format!("Читает отношения текущей базы страницами по {XID_PAGE_SIZE} OID (не более {XID_MAX_SECTION_ROWS} строк); итог сортируется по `age(relfrozenxid)`. SQL ниже — исходный шаблон, не полный запрос отдельной страницы."),
            &[
                "relation_oid",
                "schema_name / relation_name / relation_type",
                "xid_age",
                "mxid_age",
                "effective_multixact_freeze_max_age",
                "pct_of_multixact_freeze_max",
                "effective_freeze_max_age",
                "freeze_max_source",
                "pct_of_freeze_max",
                "total_size_bytes / estimated_rows",
                "heap_pages / all_visible_pages / all_visible_pct",
                "all_frozen_pages",
                "effective_freeze_min_age / freeze_min_source",
                "effective_freeze_table_age / freeze_table_source",
                "last_vacuum / last_autovacuum / last_analyze / last_autoanalyze",
                "vacuum/autovacuum/analyze/autoanalyze counters",
                "modifications_since_analyze",
                "reloptions",
                "parent relation / type / parent_xid_age для TOAST",
            ],
            &[
                "`relation_type=t` означает TOAST: проверяем связь с основной таблицей.",
                "`freeze_max_source=table` означает индивидуальную настройку таблицы.",
                "`all_frozen_pages` известно только в PostgreSQL 18; NULL в PG16/17 означает отсутствие метрики, а не ноль страниц.",
                "SQL содержит маркер `/* pgxray:xid-page-relations */`; Collector заменяет его условием `c.oid > last_oid`, `ORDER BY c.oid LIMIT` на каждую страницу.",
                "При ошибке страницы или превышении лимита вся секция недоступна; неполная выборка не выдаётся за полную.",
            ],
            RELATIONS_SQL,
        ),
        query(
            "autovacuum-triggers",
            "Условия обычного autovacuum",
            &format!("Рассчитывает пороги по мёртвым версиям и вставкам для отношений, считываемых страницами по {XID_PAGE_SIZE} OID (предел {XID_MAX_SECTION_ROWS} строк). Ниже показан SQL-шаблон до постраничного ограничения."),
            &[
                "estimated_rows / live / dead",
                "inserts_since_vacuum",
                "cumulative INSERT / UPDATE / DELETE / HOT",
                "effective threshold и scale factor",
                "estimated_dead_tuple_trigger / estimated_insert_trigger",
                "thresholds_reliable",
                "dead_trigger_reached",
                "insert_trigger_reached",
                "autovacuum_enabled",
                "stats_reset",
            ],
            &[
                "Обычные пороги по изменениям строк и принудительный VACUUM от переполнения XID/MultiXact — разные механизмы.",
                "`thresholds_reliable=false` означает, что оценка `reltuples` недостаточна для надёжного расчёта; NULL-порог нельзя трактовать как ноль.",
                "INSERT-порог учитывает долю незамороженных страниц на PostgreSQL 18; отрицательная базовая настройка отключает этот обычный механизм.",
                "`stats_reset` необходим для подтверждения непрерывности накопительных счётчиков между снимками.",
                "SQL содержит маркер `/* pgxray:xid-page-triggers */`; Collector применяет `c.oid > last_oid` и `LIMIT` отдельно к каждой странице.",
            ],
            AUTOVACUUM_TRIGGERS_SQL,
        ),
        query(
            "backends",
            "Процессы с удерживаемым backend_xmin",
            "Ищет процессы с `backend_xmin` без сохранения полного SQL выполняемого запроса.",
            &[
                "pid",
                "database",
                "username",
                "application_name",
                "state",
                "xmin_age",
                "transaction_age_seconds",
                "wait_event_type / wait_event",
            ],
            &[
                "Большой возраст xmin вместе с долгой транзакцией — основание проверить возможное удержание горизонта очистки.",
                "Это диагностический сигнал, а не рекомендация завершать процесс.",
            ],
            BACKENDS_SQL,
        ),
        query(
            "slots",
            "Слоты репликации",
            "Показывает слоты с известными значениями `xmin` или `catalog_xmin`.",
            &[
                "slot_name",
                "slot_type",
                "database",
                "active",
                "xmin_age",
                "catalog_xmin_age",
            ],
            &[
                "Особого внимания требует большой `catalog_xmin_age` логического слота.",
                "Перед изменением слота нужно выяснить его владельца и назначение.",
            ],
            SLOTS_SQL,
        ),
        query(
            "prepared",
            "Подготовленные транзакции",
            "Фиксирует количество подготовленных транзакций, возраст старейшей и до 10 примеров для расследования.",
            &[
                "count",
                "oldest_age_seconds",
                "examples (gid / xid / database / owner / prepared_at / age_seconds)",
            ],
            &[
                "Нулевое число подготовленных транзакций — обычная ситуация; наличие само по себе не доказывает проблему.",
                "Давно подготовленная транзакция может удерживать горизонт очистки; список примеров ограничен десятью записями.",
            ],
            PREPARED_SQL,
        ),
        query(
            "vacuums",
            "Текущий VACUUM",
            "Показывает выполняющиеся VACUUM в текущей базе и диагностические признаки защитного anti-wraparound обслуживания.",
            &[
                "pid",
                "relation_oid",
                "relation",
                "phase",
                "xid_age",
                "heap blocks scanned / total",
                "scan_pct",
                "backend_type",
                "is_wraparound_vacuum",
                "wraparound_detection",
                "wait_event",
            ],
            &[
                "`scan_pct` — прогресс сканирования heap, а не процент завершения всей операции VACUUM.",
                "`wraparound_detection=unknown` означает, что текст активности не позволяет достоверно распознать защитный VACUUM; `is_wraparound_vacuum=false` не всегда доказывает его отсутствие.",
            ],
            VACUUMS_SQL,
        ),
    ]
}

fn source_reference() -> Vec<XidSourceReference> {
    vec![
        source(
            "routine-vacuuming",
            "Routine Vacuuming / transaction ID wraparound",
            "Официальная методология заморозки XID и MultiXact, принудительного VACUUM и расчётов обычных порогов autovacuum.",
            "routine-vacuuming.html",
        ),
        source(
            "autovacuum-settings",
            "Automatic Vacuuming settings",
            "Описание global GUC: freeze max age, workers, dead-tuple и insert thresholds, cost settings.",
            "runtime-config-autovacuum.html",
        ),
        source(
            "table-storage-parameters",
            "CREATE TABLE / storage parameters",
            "Per-table и toast.* overrides для autovacuum_freeze_min_age, autovacuum_freeze_table_age и autovacuum_freeze_max_age.",
            "sql-createtable.html#SQL-CREATETABLE-STORAGE-PARAMETERS",
        ),
        source(
            "pg-class",
            "pg_class",
            "Определение `relfrozenxid`, `relminmxid`, `relkind`, `reltuples`, `relpages`, `relallvisible`, `relallfrozen` (PG18), `reltoastrelid` и `reloptions`.",
            "catalog-pg-class.html",
        ),
        source(
            "system-info",
            "System Information Functions",
            "`pg_current_snapshot()`, `pg_snapshot_xmax()` (read-only позиция XID), `mxid_age()` и `pg_options_to_table()`.",
            "functions-info.html",
        ),
        source(
            "statistics",
            "Monitoring Statistics",
            "`pg_stat_activity`, `pg_stat_database`, `pg_stat_all_tables`, VACUUM/ANALYZE timestamps и cumulative statistics.",
            "monitoring-stats.html",
        ),
        source(
            "vacuum-progress",
            "VACUUM progress reporting",
            "`pg_stat_progress_vacuum` и значения фаз/счётчиков progress view.",
            "progress-reporting.html#VACUUM-PROGRESS-REPORTING",
        ),
        source(
            "replication-slots",
            "pg_replication_slots",
            "Поля `xmin` и `catalog_xmin`, которые могут удерживать transaction horizon.",
            "view-pg-replication-slots.html",
        ),
        source(
            "prepared-xacts",
            "pg_prepared_xacts",
            "Prepared transactions и время их подготовки.",
            "view-pg-prepared-xacts.html",
        ),
    ]
}

fn source(id: &str, title: &str, purpose: &str, path: &str) -> XidSourceReference {
    XidSourceReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        path: path.to_owned(),
    }
}

fn method_step(id: &str, title: &str, purpose: &str, how_to_read: &[&str]) -> XidMethodStep {
    XidMethodStep {
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
) -> XidQueryReference {
    XidQueryReference {
        id: id.to_owned(),
        title: title.to_owned(),
        purpose: purpose.to_owned(),
        output: owned_strings(output),
        how_to_read: owned_strings(how_to_read),
        sql: sql.trim().to_owned(),
    }
}

fn owned_strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}
