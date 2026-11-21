use super::model::*;

pub(super) fn interpret_snapshot(
    overview: &AutovacuumOverview,
    tables: &AutovacuumSection<Vec<AutovacuumTable>>,
    vacuums: &AutovacuumSection<Vec<AutovacuumVacuumProgress>>,
    analyzes: &AutovacuumSection<Vec<AutovacuumAnalyzeProgress>>,
) -> AutovacuumInterpretation {
    let mut items = Vec::new();

    let mut limitations =
        vec![
            crate::i18n::text("n_live_tup, n_dead_tup и reltuples — приблизительные статистические оценки, а не измерение физического разрастания таблицы (bloat).", "n_live_tup, n_dead_tup, and reltuples are approximate statistics, not measurements of physical table bloat.").into(),
            crate::i18n::text("Достижение расчётного порога не гарантирует немедленный запуск автоматического VACUUM или ANALYZE.", "Reaching the calculated threshold does not guarantee an immediate automatic VACUUM or ANALYZE.").into(),
            crate::i18n::text("Защита от переполнения XID рассматривается отдельно: не уменьшайте пороги VACUUM только ради изменения показателей XID.", "XID wraparound prevention is handled separately: do not lower VACUUM thresholds merely to change XID metrics.").into(),
        ];

    if overview.stats_reset.is_none() {
        limitations.push(
            "Время stats_reset неизвестно: сравнение накопительных счётчиков и расчёт скоростей по истории не считаются надёжными."
                .into(),
        );
    }

    limitations.push(
        "Общий сбор проверяет все пользовательские таблицы и вычисляет их размеры. На больших базах он может быть медленным или завершиться по лимиту времени либо объёма ответа."
            .into(),
    );

    limitations.push(
        "Общие настройки, таблицы и выполняющиеся VACUUM/ANALYZE собираются последовательными запросами только для чтения: это не атомарный снимок одного момента времени."
            .into(),
    );

    if !overview.track_counts {
        items.push(
            interpretation_item(
                "track-counts-off",
                "track_counts выключен",
                vec![
                    "track_counts = off".into(),
                ],
                vec![
                    "При выключенном track_counts статистика изменений таблиц недостоверна; по ней нельзя оценивать достижение порогов или частоту обслуживания."
                        .into(),
                ],
                vec![
                    "Проверь настройки track_counts до изменения параметров обслуживания отдельных таблиц."
                        .into(),
                ],
            ),
        );
    }

    if !overview.autovacuum {
        items.push(
            interpretation_item(
                "autovacuum-off",
                "Обычный autovacuum глобально выключен",
                vec![
                    "autovacuum = off".into(),
                ],
                vec![
                    "Обычные автоматические VACUUM и ANALYZE не планируются; защитный VACUUM против переполнения XID работает по особым правилам."
                        .into(),
                ],
                vec![
                    "Сначала выясни, почему Autovacuum выключен на сервере, и только потом рассматривай настройки отдельных таблиц."
                        .into(),
                ],
            ),
        );
    }

    if let Some(rows) = tables.data.as_ref() {
        let dead_reached = rows.iter().filter(|row| row.dead_trigger_reached).count();

        let insert_reached = rows.iter().filter(|row| row.insert_trigger_reached).count();

        let analyze_reached = rows
            .iter()
            .filter(|row| row.analyze_trigger_reached)
            .count();

        let disabled = rows.iter().filter(|row| !row.autovacuum_enabled).count();

        if overview.track_counts && (dead_reached > 0 || insert_reached > 0 || analyze_reached > 0)
        {
            items.push(
                interpretation_item(
                    "thresholds-reached",
                    "Есть таблицы, достигшие расчётных порогов",
                    vec![
                        format!("Порог VACUUM по мёртвым строкам: {dead_reached}"),
                        format!("Порог VACUUM по вставкам: {insert_reached}"),
                        format!("Порог ANALYZE: {analyze_reached}"),
                    ],
                    vec![
                        "Эти таблицы стоит проверить: достижение порога само по себе не доказывает задержку обслуживания."
                            .into(),
                    ],
                    vec![
                        "Открой сведения о таблицах с наиболее превышенными порогами: проверь последние операции VACUUM/ANALYZE, активные процессы и действующие настройки."
                            .into(),
                    ],
                ),
            );
        }

        if !overview.track_counts {
            limitations.push(
                "track_counts выключен: выводы о достижении порогов VACUUM/ANALYZE по счётчикам таблиц пропущены; это не означает, что пороги не достигнуты."
                    .into(),
            );
        }

        if disabled > 0 {
            items.push(
                interpretation_item(
                    "tables-disabled",
                    "Для части таблиц обычный autovacuum выключен",
                    vec![
                        format!("Обычный Autovacuum выключен для {disabled} таблиц"),
                    ],
                    vec![
                        "Это может быть осознанная настройка. Обычные правила регулярности автоматического обслуживания к этим таблицам неприменимы."
                            .into(),
                    ],
                    vec![
                        "Проверь индивидуальные настройки таблиц (reloptions) и способ их обслуживания."
                            .into(),
                    ],
                ),
            );
        }
    }

    if let Some(rows) = vacuums.data.as_ref() {
        let auto = rows.iter().filter(|row| row.is_autovacuum).count();

        if auto > 0 {
            items.push(
                interpretation_item(
                    "active-autovacuum",
                    "Сейчас выполняется autovacuum",
                    vec![
                        format!("Активных автоматических VACUUM в текущей базе: {auto}"),
                    ],
                    vec![
                        "Это операции, выполнявшиеся в момент сбора. Наличие процесса не показывает, когда завершится весь VACUUM."
                            .into(),
                    ],
                    vec![
                        "Проверь фазу, прогресс сканирования таблицы и событие ожидания (wait_event) процесса."
                            .into(),
                    ],
                ),
            );
        }
    }

    if let Some(rows) = analyzes.data.as_ref() {
        let auto = rows.iter().filter(|row| row.is_autoanalyze).count();

        if auto > 0 {
            items.push(
                interpretation_item(
                    "active-autoanalyze",
                    "Сейчас выполняется autoanalyze",
                    vec![
                        format!("Активных автоматических ANALYZE в текущей базе: {auto}"),
                    ],
                    vec![
                        "VACUUM и ANALYZE имеют разные пороги. Прогресс ANALYZE нельзя оценивать по числу мёртвых строк."
                            .into(),
                    ],
                    vec![
                        "Проверь число изменений со времени последнего ANALYZE и расчётный порог для этой таблицы."
                            .into(),
                    ],
                ),
            );
        }
    }

    if let Some(error) = &tables.error {
        limitations.push(format!(
            "Статистика таблиц недоступна: нельзя оценить пороги VACUUM/ANALYZE. Причина: {error}"
        ));
    }
    if let Some(error) = &vacuums.error {
        limitations.push(format!(
            "Данные о выполняющихся VACUUM недоступны: отсутствие процессов не подтверждено. Причина: {error}"
        ));
    }
    if let Some(error) = &analyzes.error {
        limitations.push(format!(
            "Данные о выполняющихся ANALYZE недоступны: отсутствие процессов не подтверждено. Причина: {error}"
        ));
    }

    AutovacuumInterpretation { items, limitations }
}

pub(super) fn interpret_table(
    table: &AutovacuumTable,
    indexes: &AutovacuumSection<Vec<AutovacuumIndex>>,
    track_counts: Option<bool>,
) -> AutovacuumInterpretation {
    let mut items = Vec::new();

    let mut limitations =
        vec![
            crate::i18n::text("Профиль нагрузки основан на накопительных счётчиках pg_stat_user_tables за период статистики, а не на текущей скорости запросов.", "The workload profile uses cumulative pg_stat_user_tables counters for the statistics period, not the current query rate.").into(),
            crate::i18n::text("Процент мёртвых строк — статистическая оценка, а не процент физического разрастания таблицы (bloat).", "The dead tuple percentage is a statistical estimate, not a measure of physical table bloat.").into(),
        ];

    if track_counts != Some(true) {
        limitations.push(match track_counts {
            Some(false) => "track_counts выключен: выводы по порогам, HOT, TOAST и профилю нагрузки недостоверны и пропущены.",
            None => "Не удалось подтвердить track_counts: выводы по порогам, HOT, TOAST и профилю нагрузки пропущены до проверки статистики.",
            Some(true) => unreachable!(),
        }.into());
    }
    if let Some(error) = &indexes.error {
        limitations.push(format!(
            "Сведения об индексах недоступны; анализ HOT ограничен. Причина: {error}"
        ));
    }

    if !table.autovacuum_enabled && table.autovacuum_applicable {
        items.push(interpretation_item(
            "table-autovacuum-disabled",
            "Обычный Autovacuum для таблицы выключен",
            vec![format!(
                "{}.{}: autovacuum_enabled = false",
                table.schema_name, table.relation_name,
            )],
            vec![
                "Достижение расчётных порогов не запустит обычный автоматический VACUUM для этой таблицы."
                    .into(),
            ],
            vec!["Проверь индивидуальные настройки таблицы и то, как она обслуживается вручную.".into()],
        ));
    }

    if track_counts == Some(true) && table.dead_trigger_reached {
        items.push(
            interpretation_item(
                "dead-trigger-reached",
                "Достигнут порог VACUUM по мёртвым строкам",
                vec![
                    format!("Мёртвых строк (n_dead_tup): {}", table.dead_tuples),
                    format!("Расчётный порог VACUUM: {}", table.dead_tuple_trigger),
                    format!(
                        "Доля использованного порога: {}",
                        format_pct(table.dead_trigger_used_pct),
                    ),
                ],
                vec![
                    "Число мёртвых строк достигло расчётного порога VACUUM. Это не означает, что процесс уже должен быть виден в снимке."
                        .into(),
                ],
                vec![
                    "Проверь активные процессы VACUUM, дату последнего автоматического обслуживания и историю снимков."
                        .into(),
                ],
            ),
        );
    }

    if track_counts == Some(true) && table.insert_trigger_reached {
        items.push(
            interpretation_item(
                "insert-trigger-reached",
                "Достигнут порог VACUUM по вставкам",
                vec![
                    format!("Вставок после VACUUM: {}", table.inserts_since_vacuum),
                    format!(
                        "Расчётный порог по вставкам: {}",
                        table.insert_trigger
                            .map(|value| value.to_string())
                            .unwrap_or_else(|| "выключено".into()),
                    ),
                ],
                vec![
                    "При преобладании вставок PostgreSQL может запускать VACUUM даже при небольшом числе мёртвых строк."
                        .into(),
                ],
                vec![
                    "Сопоставь сигнал с характером вставок и отдельно проверь требования защиты от переполнения XID."
                        .into(),
                ],
            ),
        );
    }

    if track_counts == Some(true) && table.analyze_trigger_reached {
        items.push(interpretation_item(
            "analyze-trigger-reached",
            "Достигнут порог автоматического ANALYZE",
            vec![
                format!(
                    "Изменений после ANALYZE: {}",
                    table.modifications_since_analyze,
                ),
                format!("Расчётный порог ANALYZE: {}", table.analyze_trigger),
            ],
            vec!["Частоту обновления статистики планировщика ANALYZE оценивают отдельно от частоты VACUUM.".into()],
            vec!["Проверь дату последнего автоматического ANALYZE и историю числа его запусков.".into()],
        ));
    }

    if track_counts == Some(true)
        && table.tuples_updated > 0
        && table
            .hot_update_pct
            .is_some_and(|pct| pct.is_finite() && pct < 5.0)
    {
        let index_count = indexes
            .data
            .as_ref()
            .map(|rows| rows.len())
            .unwrap_or(table.index_count as usize);

        items.push(
            interpretation_item(
                "low-hot",
                "Доля HOT-обновлений очень мала",
                vec![
                    format!("Обновлений строк: {}", table.tuples_updated),
                    format!("Доля HOT: {}", format_pct(table.hot_update_pct)),
                    format!("Индексов: {index_count}"),
                ],
                vec![
                    "Малая доля HOT может сопровождаться дополнительными обновлениями индексов, но не доказывает проблему. Изменение индексируемых столбцов может исключать HOT независимо от fillfactor."
                        .into(),
                ],
                vec![
                    "Сравни изменяемые UPDATE столбцы с индексируемыми. Рассматривай fillfactor лишь после выяснения причин."
                        .into(),
                ],
            ),
        );
    }

    if let (Some(true), Some(toast)) = (track_counts, table.toast.as_ref()) {
        if toast.dead_trigger_reached || toast.insert_trigger_reached {
            items.push(
                interpretation_item(
                    "toast-trigger-reached",
                    "TOAST достиг расчётного порога VACUUM",
                    vec![
                        format!(
                            "{}.{}: dead {} / trigger {}; inserts since vacuum {} / trigger {}",
                            toast.schema_name,
                            toast.relation_name,
                            toast.dead_tuples,
                            toast.dead_tuple_trigger,
                            toast.inserts_since_vacuum,
                            toast.insert_trigger
                                .map(|value| value.to_string())
                                .unwrap_or_else(|| "выключено".into()),
                        ),
                    ],
                    vec![
                        "TOAST обслуживается как отдельное отношение и может иметь собственные настройки autovacuum. Хорошее состояние основной таблицы не заменяет проверку TOAST."
                            .into(),
                    ],
                    vec![
                        "Проверь настройки TOAST, последний автоматический VACUUM и выполняющиеся операции обслуживания."
                            .into(),
                    ],
                ),
            );
        }
    }

    if track_counts == Some(true)
        && table.workload_profile == "append_only_observed"
        && table.dead_tuples == 0
    {
        items.push(
            interpretation_item(
                "append-only",
                "Наблюдается преимущественно добавление строк",
                vec![
                    format!("Вставок: {}", table.tuples_inserted),
                    "Обновлений: 0; удалений: 0; мёртвых строк (оценка): 0".into(),
                ],
                vec![
                    "В такой таблице порог по мёртвым строкам может достигаться редко; обслуживание может запускаться по числу вставок."
                        .into(),
                ],
                vec![
                    "Проверь порог по вставкам и отдельно оцени необходимость VACUUM для защиты от переполнения XID."
                        .into(),
                ],
            ),
        );
    }

    if table.relation_type == "partitioned_table" {
        items.push(
            interpretation_item(
                "partitioned-parent",
                "Секционированный родитель обслуживается иначе, чем таблица с данными",
                vec![
                    format!(
                        "{}.{} — родитель секционированной таблицы",
                        table.schema_name,
                        table.relation_name,
                    ),
                ],
                vec![
                    "PostgreSQL обслуживает физические партиции автоматически, но не запускает обычный Autovacuum/Autoanalyze для родительской секционированной таблицы, поскольку она не хранит строки напрямую."
                        .into(),
                ],
                vec![
                    "Проверь физические партиции отдельно. При необходимости обнови статистику родителя вручную через ANALYZE."
                        .into(),
                ],
            ),
        );

        limitations.push(
            "Расчётные пороги секционированного родителя приведены для справки и не означают, что его будет автоматически обслуживать Autovacuum."
                .into(),
        );
    }

    AutovacuumInterpretation { items, limitations }
}

fn interpretation_item(
    id: &str,
    title: &str,
    observed: Vec<String>,
    meaning: Vec<String>,
    next_steps: Vec<String>,
) -> AutovacuumInterpretationItem {
    let severity = match id {
        "track-counts-off" | "autovacuum-off" | "tables-disabled" | "table-autovacuum-disabled" => {
            AutovacuumInterpretationSeverity::Warning
        }
        "thresholds-reached"
        | "dead-trigger-reached"
        | "insert-trigger-reached"
        | "analyze-trigger-reached"
        | "toast-trigger-reached"
        | "low-hot" => AutovacuumInterpretationSeverity::Attention,
        _ => AutovacuumInterpretationSeverity::Info,
    };
    AutovacuumInterpretationItem {
        id: id.to_owned(),
        title: title.to_owned(),
        severity,
        observed,
        meaning,
        next_steps,
    }
}

fn format_pct(value: Option<f64>) -> String {
    value
        .map(|value| format!("{value:.1}%"))
        .unwrap_or_else(|| "нет данных".into())
}
