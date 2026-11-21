use super::model::*;

pub(super) fn interpret_snapshot(
    overview: &PgssOverview,
    stats: &PgssSection<PgssStatsInfo>,
    capacity: &PgssSection<PgssCapacity>,
    role_database: &PgssSection<Vec<PgssRoleDatabaseCardinality>>,
    commands: &PgssSection<Vec<PgssCommandCardinality>>,
    call_targets: &PgssSection<Vec<PgssCallTargetCardinality>>,
    performance: &PgssSection<PgssPerformanceSamples>,
) -> PgssInterpretation {
    let mut items = Vec::new();

    let mut limitations = Vec::new();

    if !overview.extension_installed {
        items.push(
            interpretation_item(
                "extension-not-installed",
                crate::i18n::text("pg_stat_statements не установлен в текущей базе", "pg_stat_statements is not installed in the current database"),
                vec![
                    format!(
                        "database = {}; extension_installed = false",
                        overview.database,
                    ),
                ],
                vec![
                    crate::i18n::text("Без extension view pgXray не может собрать PGSS health, cardinality и performance statistics в этой базе.", "Without the extension view, pgXray cannot collect PGSS health, cardinality, or performance statistics in this database.")
                        .into(),
                    crate::i18n::text("Это не означает, что library обязательно не загружена на уровне сервера.", "This does not necessarily mean the library is not loaded at the server level.")
                        .into(),
                ],
                vec![
                    crate::i18n::text("Проверить, должна ли эта база предоставлять pg_stat_statements для диагностики.", "Verify whether this database should expose pg_stat_statements for diagnostics.")
                        .into(),
                    crate::i18n::text("Не выполнять CREATE EXTENSION автоматически: это отдельное административное изменение.", "Do not execute CREATE EXTENSION automatically; this is a separate administrative change.")
                        .into(),
                ],
            ),
        );
    }

    if overview.extension_installed && !overview.preloaded {
        items.push(
            interpretation_item(
                "preload-not-detected",
                crate::i18n::text("Extension установлен, но pg_stat_statements не найден в shared_preload_libraries", "The extension is installed, but pg_stat_statements is not in shared_preload_libraries"),
                vec![
                    format!(
                        "shared_preload_libraries = '{}'",
                        overview.shared_preload_libraries,
                    ),
                ],
                vec![
                    crate::i18n::text("pg_stat_statements требует загрузки через shared_preload_libraries, поэтому часть запросов может быть недоступна или неактивна.", "pg_stat_statements must be loaded through shared_preload_libraries, so some queries may be unavailable or inactive.")
                        .into(),
                ],
                vec![
                    crate::i18n::text("Проверить фактическую конфигурацию сервера и ошибки секций snapshot.", "Check the actual server configuration and errors in snapshot sections.")
                        .into(),
                    crate::i18n::text("Не менять postmaster settings из pgXray на этом этапе.", "Do not change postmaster settings from pgXray at this stage.")
                        .into(),
                ],
            ),
        );
    }

    if let (Some(stats), Some(capacity)) = (stats.data.as_ref(), capacity.data.as_ref()) {
        items.push(interpretation_item(
            "health",
            crate::i18n::text(
                "Состояние хранилища pg_stat_statements",
                "pg_stat_statements store health",
            ),
            vec![
                format!(
                    "entries = {} / {} ({:.1}%)",
                    capacity.current_entries, capacity.max_entries, capacity.fill_pct,
                ),
                format!(
                    "dealloc = {} since stats_reset = {}",
                    stats.dealloc,
                    stats.stats_reset.as_deref().unwrap_or("<unknown>"),
                ),
                format!(
                    "one-call entries = {} ({:.1}%); calls<=5 = {} ({:.1}%)",
                    capacity.one_call_entries,
                    capacity.one_call_pct,
                    capacity.up_to_5_calls_entries,
                    capacity.up_to_5_calls_pct,
                ),
            ],
            vec![
                crate::i18n::text(
                    "Высокая заполненность сама по себе не является проблемой.",
                    "High utilization is not necessarily a problem.",
                )
                .into(),
                dealloc_snapshot_meaning(stats.dealloc),
            ],
            vec![
                crate::i18n::text(
                    "Сравнить этот snapshot со следующим и смотреть dealloc delta/rate.",
                    "Compare this snapshot with the next and examine the dealloc delta and rate.",
                )
                .into(),
                crate::i18n::text(
                    "Если dealloc растёт, локализовать cardinality по role/database/command.",
                    "If dealloc increases, examine cardinality by role, database, and command.",
                )
                .into(),
            ],
        ));
    }

    if let Some(top) = role_database.data.as_ref().and_then(|rows| rows.first()) {
        items.push(
            interpretation_item(
                "top-cardinality-owner",
                crate::i18n::text("Крупнейший владелец statement cardinality", "Largest contributor to statement cardinality"),
                vec![
                    format!(
                        "role={} database={} entries={} ({:.1}% всех entries), one-call={} ({:.1}%)",
                        top.role_name
                            .as_deref()
                            .unwrap_or("<unknown>"),
                        top.database
                            .as_deref()
                            .unwrap_or("<unknown>"),
                        top.entries,
                        top.entries_pct,
                        top.one_call_entries,
                        top.one_call_pct,
                    ),
                ],
                vec![
                    crate::i18n::text("Это крупнейшая role/database группа текущего snapshot.", "This is the largest role/database group in the current snapshot.")
                        .into(),
                    crate::i18n::text("Большое число one-shot entries делает эту группу приоритетным кандидатом для расследования cardinality, но не доказывает причину dealloc без временной delta.", "A large number of one-shot entries makes this group a priority for cardinality investigation, but does not establish the cause of dealloc without a time delta.")
                        .into(),
                ],
                vec![
                    crate::i18n::text("Посмотреть распределение command для этой role/database.", "Inspect the command distribution for this role/database.")
                        .into(),
                    crate::i18n::text("Сопоставить с dealloc delta после следующего snapshot.", "Compare with the dealloc delta after the next snapshot.")
                        .into(),
                ],
            ),
        );
    }

    if commands
        .data
        .as_ref()
        .is_some_and(|rows| rows.iter().any(|row| row.command == "<HIDDEN>"))
    {
        limitations.push(
            "Часть текстов SQL недоступна из-за прав: <HIDDEN> не означает отсутствующий тип команды. Распределение по типам неполно.".into(),
        );
    }
    if commands
        .data
        .as_ref()
        .is_some_and(|rows| rows.iter().any(|row| row.command == "<WITH/CTE>"))
    {
        limitations.push(
            "WITH/CTE сгруппирован отдельно: первый токен WITH не позволяет достоверно определить конечный SELECT/INSERT/UPDATE/DELETE.".into(),
        );
    }

    if let Some(top) = commands.data.as_ref().and_then(|rows| rows.first()) {
        items.push(
            interpretation_item(
                "top-command",
                crate::i18n::text("Крупнейший command-класс", "Largest command class"),
                vec![
                    format!(
                        "{}: role={} database={} entries={} ({:.1}% всех entries), one-call={} ({:.1}%)",
                        top.command,
                        top.role_name
                            .as_deref()
                            .unwrap_or("<unknown>"),
                        top.database
                            .as_deref()
                            .unwrap_or("<unknown>"),
                        top.entries,
                        top.entries_pct,
                        top.one_call_entries,
                        top.one_call_pct,
                    ),
                ],
                vec![
                    "Command distribution помогает понять, какой тип statement создаёт cardinality, не выводя полный SQL."
                        .into(),
                ],
                vec![
                    if top.command == "CALL" {
                        "Если CALL доминирует, проверить track_utility и CALL target aggregation."
                            .into()
                    } else {
                        "Сопоставить этот command с role/database cardinality и workload приложения."
                            .into()
                    },
                ],
            ),
        );
    }

    if overview.track_utility == Some(true) {
        if let Some(target) = call_targets.data.as_ref().and_then(|rows| rows.first()) {
            if target.entries_pct_of_max >= 90.0 && target.one_call_pct >= 90.0 {
                items.push(
                    interpretation_item(
                        "dominant-call-target",
                        "Одна текстовая группа CALL занимает почти весь configured PGSS capacity",
                        vec![
                            format!(
                                "call_tag={} role={} database={} entries={} ({:.1}% pg_stat_statements.max), one-call={:.1}%",
                                target.call_tag,
                                target.role_name
                                    .as_deref()
                                    .unwrap_or("<unknown>"),
                                target.database
                                    .as_deref()
                                    .unwrap_or("<unknown>"),
                                target.entries,
                                target.entries_pct_of_max,
                                target.one_call_pct,
                            ),
                            "pg_stat_statements.track_utility = on"
                                .into(),
                        ],
                        vec![
                            "Такой профиль указывает на высокую cardinality текстовой группы CALL, но не доказывает идентичность процедуры."
                                .into(),
                            "Он локализует cardinality, но конкретную клиентскую причину — например client-side parameter binding — нужно подтвердить по приложению/driver."
                                .into(),
                        ],
                        vec![
                            "Определить driver и способ формирования проблемного CALL."
                                .into(),
                            "Не отключать track_utility и не увеличивать pg_stat_statements.max автоматически."
                                .into(),
                            "Проверить dealloc delta после изменения клиента или согласованного mitigation."
                                .into(),
                        ],
                    ),
                );
            }
        }
    }

    if let Some(samples) = performance.data.as_ref() {
        let mut observed = Vec::new();

        push_performance_fact(
            &mut observed,
            "TOP total_exec_time",
            samples.total_exec.first(),
            |row| {
                format!(
                    "{} ms total / {} calls / queryid={}",
                    round_1(row.total_exec_time_ms,),
                    row.calls,
                    queryid_label(row),
                )
            },
        );

        push_performance_fact(
            &mut observed,
            "TOP mean_exec_time",
            samples.mean_exec.first(),
            |row| {
                format!(
                    "{} ms mean / max {} ms / queryid={}",
                    round_3(row.mean_exec_time_ms,),
                    round_3(row.max_exec_time_ms,),
                    queryid_label(row),
                )
            },
        );

        push_performance_fact(
            &mut observed,
            "TOP temp I/O",
            samples.temp_io.first(),
            |row| {
                format!(
                    "{} temp blocks written / {} read / queryid={}",
                    row.temp_blks_written,
                    row.temp_blks_read,
                    queryid_label(row),
                )
            },
        );

        push_performance_fact(
            &mut observed,
            "TOP shared reads",
            samples.shared_reads.first(),
            |row| {
                format!(
                    "{} shared blocks read / queryid={}",
                    row.shared_blks_read,
                    queryid_label(row),
                )
            },
        );

        push_performance_fact(&mut observed, "TOP WAL", samples.wal.first(), |row| {
            format!(
                "{} WAL bytes / queryid={}",
                round_1(row.wal_bytes,),
                queryid_label(row),
            )
        });

        push_performance_fact(
            &mut observed,
            "TOP execution variance",
            samples.variance.first(),
            |row| {
                format!(
                    "stddev {} ms / mean {} ms / queryid={}",
                    round_3(row.stddev_exec_time_ms,),
                    round_3(row.mean_exec_time_ms,),
                    queryid_label(row),
                )
            },
        );

        if overview.track_planning == Some(true) {
            push_performance_fact(
                &mut observed,
                "TOP planning time",
                samples.planning.first(),
                |row| {
                    format!(
                        "{} ms total planning / {} plans / queryid={}",
                        round_1(row.total_plan_time_ms,),
                        row.plans,
                        queryid_label(row),
                    )
                },
            );
        }

        if !observed.is_empty() {
            items.push(
                interpretation_item(
                    "performance-hotspots",
                    "Performance hotspots текущего PGSS snapshot",
                    observed,
                    vec![
                        "Каждая строка — только кандидат для следующего уровня анализа."
                            .into(),
                        "Большой temp I/O не доказывает неправильный work_mem; большой shared read не доказывает плохой cache hit ratio; большой WAL не является сам по себе неисправностью."
                            .into(),
                        "Mean/max/stddev описывают агрегированную историю entry с момента stats_reset или её повторного появления после deallocation."
                            .into(),
                    ],
                    vec![
                        "Открыть drill-down конкретного queryid и затем при необходимости перейти к EXPLAIN/lock/table diagnostics."
                            .into(),
                        "При активном dealloc не считать исчезновение/reappearance queryid обычным reset всей PGSS статистики."
                            .into(),
                    ],
                ),
            );
        }
    }

    if stats
        .data
        .as_ref()
        .is_some_and(|info| info.stats_reset.is_none())
    {
        limitations.push(
            "Время stats_reset для pg_stat_statements неизвестно; сравнение накопительных счётчиков без подтверждённой эпохи запрещено.".into(),
        );
    }
    if overview.track_planning != Some(true) {
        limitations.push(
            "Время планирования не подтверждено: pg_stat_statements.track_planning не включён или недоступен; нули planning не означают отсутствие затрат.".into(),
        );
    }
    if !overview.track_io_timing {
        limitations.push(
            "track_io_timing=off: нулевые blk_read_time/blk_write_time не означают отсутствие I/O."
                .into(),
        );
    }
    limitations.push(
        "Performance history_sample — ограниченная TOP-выборка (до 300 entries, только с queryid). Отсутствие statement в выборке не означает отсутствие SQL на сервере.".into(),
    );
    limitations.push(
        "CALL target — хэш текстовой метки, а не достоверный OID процедуры; без доступа к SQL text выборка неполна.".into(),
    );

    push_section_limitation(&mut limitations, "pg_stat_statements_info", stats);

    push_section_limitation(&mut limitations, "Capacity", capacity);

    push_section_limitation(&mut limitations, "Role/database cardinality", role_database);

    push_section_limitation(&mut limitations, "Command cardinality", commands);

    push_section_limitation(&mut limitations, "CALL targets", call_targets);

    push_section_limitation(&mut limitations, "Performance", performance);

    limitations.push(
        "Один snapshot не показывает текущую скорость dealloc. Для churn нужен второй snapshot с тем же stats_reset."
            .into(),
    );

    limitations.push(
        "pg_stat_statements хранит агрегированные entries. При deallocation редкий queryid может исчезнуть и позже появиться снова с новыми накопительными счётчиками."
            .into(),
    );

    limitations.push(
        "Broad cardinality/performance collectors не возвращают полный SQL. Query text запрашивается отдельно только через drill-down выбранного statement."
            .into(),
    );

    PgssInterpretation { items, limitations }
}

pub(super) fn dealloc_snapshot_meaning(dealloc: i64) -> String {
    if dealloc == 0 {
        "dealloc = 0: после stats_reset deallocation events не наблюдались; для оценки текущей скорости churn всё равно нужен второй snapshot.".into()
    } else {
        "dealloc > 0: после stats_reset наблюдались deallocation events, но абсолютный счётчик без второго snapshot не показывает текущую скорость churn.".into()
    }
}

fn push_performance_fact<F>(
    target: &mut Vec<String>,
    label: &str,
    row: Option<&PgssPerformanceRow>,
    render: F,
) where
    F: FnOnce(&PgssPerformanceRow) -> String,
{
    if let Some(row) = row {
        target.push(format!("{label}: {}", render(row),));
    }
}

fn queryid_label(row: &PgssPerformanceRow) -> &str {
    row.queryid.as_deref().unwrap_or("<not visible>")
}

fn round_1(value: f64) -> String {
    format!("{value:.1}")
}

fn round_3(value: f64) -> String {
    format!("{value:.3}")
}

fn interpretation_item(
    id: &str,
    title: &str,
    observed: Vec<String>,
    meaning: Vec<String>,
    next_steps: Vec<String>,
) -> PgssInterpretationItem {
    PgssInterpretationItem {
        id: id.to_owned(),
        title: title.to_owned(),
        observed,
        meaning,
        next_steps,
    }
}

fn push_section_limitation<T>(
    limitations: &mut Vec<String>,
    title: &str,
    section: &PgssSection<T>,
) {
    if let Some(error) = section.error.as_deref() {
        limitations.push(format!("{title}: секция недоступна — {error}",));
    }
}
